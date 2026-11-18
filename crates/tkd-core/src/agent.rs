use crate::config::{self, AgentPick, Channel};
use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Tarefas que chamam agente (nome = template em agents/, menos o .md).
pub const TASKS: [(&str, &str); 6] = [
    ("study", "Estudar referências"),
    ("ideate", "Criar a ideia"),
    ("review", "Avaliar o vídeo"),
    ("recreate", "Recriar edit"),
    ("footage", "Buscar imagens"),
    ("tag_clips", "Catalogar clipes"),
];

pub const MODELS: [(&str, &str); 3] = [
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-haiku-5-5", "Haiku 5.5"),
];

pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Modelo e esforco efetivos de uma chamada, e de onde vieram.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Resolved {
    pub model: String,
    pub effort: String,
    /// global | global_task | channel | channel_task
    pub model_from: String,
    pub effort_from: String,
}

/// Precedencia: padrao global -> tarefa global -> canal -> tarefa do canal (o ultimo vence).
pub fn resolve_with(global: &config::Global, ch: Option<&config::AgentCfg>, task: &str) -> Resolved {
    let mut r = Resolved {
        model: global.agents.model.clone(),
        effort: global.agents.effort.clone(),
        model_from: "global".into(),
        effort_from: "global".into(),
    };
    let mut apply = |p: &AgentPick, from: &str| {
        if !p.model.is_empty() {
            r.model = p.model.clone();
            r.model_from = from.into();
        }
        if !p.effort.is_empty() {
            r.effort = p.effort.clone();
            r.effort_from = from.into();
        }
    };
    // nome antigo da tarefa (antes do rename para ingles) ainda vale no tkd.toml/channel.toml
    let old = match task {
        "review_clips" => Some("review_cortes"),
        _ => None,
    };
    let find = |m: &std::collections::BTreeMap<String, AgentPick>| m.get(task).or_else(|| old.and_then(|o| m.get(o))).cloned();
    if let Some(p) = find(&global.tasks) {
        apply(&p, "global_task");
    }
    if let Some(c) = ch {
        apply(&AgentPick { model: c.model.clone(), effort: c.effort.clone() }, "channel");
        if let Some(p) = find(&c.tasks).as_ref() {
            apply(p, "channel_task");
        }
    }
    r
}

pub fn resolve(ch: &Channel, task: &str) -> Resolved {
    resolve_with(&config::load_global(), Some(&ch.agent), task)
}

/// Limite de chamadas de agente simultaneas (tkd.toml: agents.parallel).
fn llm_slots() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| {
        let n = config::load_global().agents.parallel.clamp(1, 8);
        *SLOTS_LIMIT.lock().unwrap_or_else(|e| e.into_inner()) = n;
        Semaphore::new(n)
    })
}

static SLOTS_LIMIT: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// Muda o limite na hora (antes o semaforo ficava com o valor do boot ate reiniciar o app).
/// Diminuir espera as chamadas em andamento terminarem para recolher as vagas.
pub fn set_parallel(n: usize) {
    let n = n.clamp(1, 8);
    let slots_sem = llm_slots();
    let mut cur = SLOTS_LIMIT.lock().unwrap_or_else(|e| e.into_inner());
    if n > *cur {
        slots_sem.add_permits(n - *cur);
    } else if n < *cur {
        let k = (*cur - n) as u32;
        tokio::spawn(async move {
            if let Ok(p) = llm_slots().acquire_many(k).await {
                p.forget();
            }
        });
    }
    *cur = n;
}

/// Substituicao simples de {{chave}} nos templates de agents/.
pub fn render_template(name: &str, vars: &HashMap<&str, String>) -> Result<String> {
    let path = config::root().join("agents").join(name);
    let mut text = std::fs::read_to_string(&path).with_context(|| format!("lendo template {}", path.display()))?;
    for (k, v) in vars {
        text = text.replace(&format!("{{{{{k}}}}}"), v);
    }
    Ok(text)
}

/// Envelope de `claude -p --output-format json`.
#[derive(Debug, Default, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub num_turns: i64,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub total_cost_usd: f64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
}

pub fn parse_envelope(stdout: &str) -> Option<Envelope> {
    // pode vir log antes; o envelope e o ultimo objeto JSON
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or(stdout);
    serde_json::from_str(line.trim()).ok().or_else(|| serde_json::from_str(stdout.trim()).ok())
}

/// Teto de turnos por tarefa. Ler as imagens em paralelo cabe em 1 turno; o resto e folga.
/// Sem teto, o recreate chegou a 25 turnos fuçando codigo-fonte (1,9 mi de tokens numa chamada).
fn max_turns(task: &str) -> u32 {
    match task {
        "recreate" => 8,
        "study" | "tag_clips" => 6,
        _ => 4,
    }
}

/// System prompt enxuto. Substitui o do Claude Code (~5 mil tokens de instrucoes de codigo).
fn system_prompt(root: &std::path::Path) -> String {
    format!(
        "Voce e um agente de um pipeline automatizado de videos (tkd), rodando sem humano.\n\
         Pasta de trabalho: {}\n\
         Caminhos relativos nas instrucoes sao relativos a ela; a ferramenta Read exige caminho absoluto, \
         entao junte a pasta de trabalho.\n\
         Use ferramentas so para ler os arquivos que a tarefa manda ler (leia varios no mesmo turno). \
         Nao explore codigo-fonte nem outras pastas: tudo o que voce precisa esta no prompt.\n\
         Quando pedirem JSON, responda so com o JSON, sem texto em volta.",
        root.display()
    )
}

/// Monta o `claude -p` isolado: sem settings do usuario/projeto (hooks caveman/orca, plugins),
/// sem MCP, sem skills, sem memoria automatica, so as ferramentas da tarefa e system prompt proprio.
/// Medido em 16/11/2026 ("responda ok"): 31 mil tokens de contexto -> 1,9 mil.
fn base_cmd(root: &std::path::Path, pick: &Resolved, tools: &[&str], turns: u32) -> Command {
    let tools = tools.join(",");
    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .args(["--output-format", "json"])
        .args(["--model", &pick.model])
        .args(["--setting-sources", ""])
        .args(["--tools", &tools])
        .arg("--strict-mcp-config")
        .arg("--disable-slash-commands")
        .args(["--system-prompt", &system_prompt(root)])
        .args(["--max-turns", &turns.to_string()])
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !tools.is_empty() {
        cmd.args(["--allowedTools", &tools]);
    }
    if !pick.effort.is_empty() {
        cmd.args(["--effort", &pick.effort]);
    }
    // sem janela e abaixo do normal: o claude -p e os filhos dele (ferramentas) cedem ao usuario
    crate::proc::background(&mut cmd);
    cmd
}

/// Roda o comando mandando o prompt pelo stdin (a linha de comando do Windows corta em 32 mil chars).
async fn run(mut cmd: Command, prompt: &str, timeout_secs: u64) -> Result<std::process::Output> {
    let fut = async {
        let plan = crate::proc::chaos_gate(&cmd).await.context("executando `claude`")?;
        // respostas simuladas saem antes de rodar: o experimento nao gasta tokens
        if let Some(out) = chaos_fake() {
            return Ok(out);
        }
        let mut child = cmd.spawn().context("executando `claude`")?;
        let tree = crate::proc::Tree::adopt(&child);
        let _chaos = crate::chaos::Killer::arm(&plan, child.id());
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.context("mandando o prompt")?;
        }
        let out = crate::proc::wait_output(child, &tree).await.context("esperando o `claude`")?;
        Ok(chaos_reply(out))
    };
    let timeout_secs = crate::chaos::limit(Duration::from_secs(timeout_secs)).as_secs().max(1);
    tokio::time::timeout(Duration::from_secs(timeout_secs), fut)
        .await
        .map_err(|_| anyhow!("claude -p estourou {timeout_secs}s"))?
}

/// Respostas do `claude -p` inventadas pelo macaco, sem rodar o claude: 429, saida vazia ou o envelope
/// JSON cortado no meio (com o texto da resposta tambem pela metade).
fn chaos_fake() -> Option<std::process::Output> {
    use crate::chaos::{hit, Kind};
    let ok = |stdout: &str| std::process::Output {
        status: std::process::ExitStatus::default(),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    };
    if hit(Kind::RateLimit, "claude") {
        return Some(crate::chaos::failed_output(
            "API Error: 429 {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\"}} (caos)",
        ));
    }
    if hit(Kind::Empty, "claude") {
        return Some(ok(""));
    }
    if hit(Kind::Truncate, "claude") {
        return Some(ok(r#"{"type":"result","subtype":"success","is_error":false,"session_id":"caos","result":"{\"title\": \"teste de ca"#));
    }
    None
}

/// Caos na resposta real (reservado: hoje todas as falhas de resposta sao simuladas sem rodar).
fn chaos_reply(out: std::process::Output) -> std::process::Output {
    out
}

/// Spawna `claude -p` e devolve o texto da resposta. Registra o consumo em `agent_calls`.
///
/// Se bater no teto de turnos sem responder, retoma a MESMA sessao (cache quente) com 1 turno
/// pedindo a resposta final, em vez de reenviar o prompt inteiro.
pub async fn ask(ch: &Channel, task: &str, prompt: &str, allowed_tools: &[&str], timeout_secs: u64) -> Result<String> {
    Ok(ask_session(ch, task, prompt, allowed_tools, timeout_secs, None).await?.0)
}

/// `ask` que pode continuar uma sessao (`resume`) e devolve tambem o id da sessao.
async fn ask_session(
    ch: &Channel,
    task: &str,
    prompt: &str,
    allowed_tools: &[&str],
    timeout_secs: u64,
    resume: Option<&str>,
) -> Result<(String, String)> {
    let pick = resolve(ch, task);
    let _slot = llm_slots().acquire().await.map_err(|_| anyhow!("fila de agentes fechada"))?;
    let span = crate::events::step(&ch.slug, "agent");
    span.action(&format!("{} ({})", task_label(task), model_label(&pick.model)), "claude");

    let root = config::root();
    tracing::info!("[{}] claude -p {task} {} / {} ({} chars)", ch.slug, pick.model, pick.effort, prompt.len());
    let t0 = Instant::now();
    let mut env = None;
    let mut stdout = String::new();
    for attempt in 0..2 {
        let (cmd, input) = if attempt == 0 {
            let mut c = base_cmd(&root, &pick, allowed_tools, max_turns(task));
            if let Some(sid) = resume.filter(|s| !s.is_empty()) {
                c.args(["--resume", sid]);
            }
            (c, prompt.to_string())
        } else {
            let Some(sid) = env.as_ref().map(|e: &Envelope| e.session_id.clone()).filter(|s| !s.is_empty()) else { break };
            tracing::warn!("[{}] {task}: bateu {} turnos sem responder; pedindo a resposta final", ch.slug, max_turns(task));
            let mut c = base_cmd(&root, &pick, allowed_tools, 1);
            c.args(["--resume", &sid]);
            (c, "Pare de ler arquivos. Responda AGORA com a resposta final no formato pedido.".to_string())
        };
        let t1 = Instant::now();
        let out = match run(cmd, &input, timeout_secs).await {
            Ok(o) => o,
            Err(e) => {
                span.fail(&e.to_string());
                return Err(e);
            }
        };
        let ms = t1.elapsed().as_millis() as i64;
        stdout = String::from_utf8_lossy(&out.stdout).to_string();
        env = parse_envelope(&stdout);
        let hit_cap = env.as_ref().is_some_and(|e| e.subtype == "error_max_turns");
        record(ch, task, &pick, env.as_ref(), ms, out.status.success() || hit_cap);
        if hit_cap {
            continue;
        }
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let msg = format!("claude -p falhou ({}): {err} {}", out.status, stdout.chars().take(300).collect::<String>());
            span.fail(&msg);
            anyhow::bail!(msg);
        }
        break;
    }
    let session = env.as_ref().map(|e| e.session_id.clone()).unwrap_or_default();
    let text = match env {
        Some(e) if !e.result.is_empty() || e.is_error => e.result,
        Some(e) if e.subtype == "error_max_turns" => String::new(),
        _ => stdout,
    };
    span.ok(&format!("{} s", t0.elapsed().as_secs()));
    Ok((text, session))
}

fn record(ch: &Channel, task: &str, pick: &Resolved, env: Option<&Envelope>, ms: i64, ok: bool) {
    let u = env.map(|e| e.usage.clone()).unwrap_or_default();
    let call = crate::state::AgentCall {
        channel: ch.slug.clone(),
        task: task.to_string(),
        model: pick.model.clone(),
        effort: pick.effort.clone(),
        input: u.input_tokens,
        output: u.output_tokens,
        cache_read: u.cache_read_input_tokens,
        cache_write: u.cache_creation_input_tokens,
        cost: env.map(|e| e.total_cost_usd).unwrap_or(0.0),
        turns: env.map(|e| e.num_turns).unwrap_or(0),
        ms,
        ok,
        source: "tkd".into(),
        session: env.map(|e| e.session_id.clone()).unwrap_or_default(),
        at: crate::state::now(),
    };
    if let Err(e) = crate::state::Store::open().and_then(|s| s.record_agent_call(&call)) {
        tracing::warn!("nao gravei o consumo: {e}");
    }
}

pub fn task_label(task: &str) -> &str {
    TASKS.iter().find(|(k, _)| *k == task).map(|(_, v)| *v).unwrap_or(task)
}

pub fn model_label(model: &str) -> String {
    MODELS
        .iter()
        .find(|(k, _)| *k == model || model.contains(&k[7..]))
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| model.to_string())
}

/// Primeiro objeto JSON do texto (tolera cercas ``` e comentario em volta).
pub fn extract_json(text: &str) -> Result<&str> {
    let t = text.trim();
    let body = if let Some(i) = t.find("```json") {
        let rest = &t[i + 7..];
        rest.split("```").next().unwrap_or(rest)
    } else if let Some(i) = t.find("```") {
        let rest = &t[i + 3..];
        rest.split("```").next().unwrap_or(rest)
    } else {
        t
    };
    let start = body.find('{').ok_or_else(|| anyhow!("sem JSON na resposta: {t:.300}"))?;
    let end = body.rfind('}').ok_or_else(|| anyhow!("JSON truncado: {t:.300}"))?;
    Ok(&body[start..=end])
}

/// Pergunta esperando JSON.
///
/// Se a resposta tem JSON mas nao valida, pede so a correcao do JSON (sem reenviar o prompt
/// inteiro nem liberar ferramentas). Se nao veio JSON nenhum, refaz a pergunta original.
pub async fn ask_json<T: DeserializeOwned>(
    ch: &Channel,
    task: &str,
    prompt: &str,
    allowed_tools: &[&str],
    timeout_secs: u64,
    retries: u32,
) -> Result<T> {
    Ok(ask_json_session(ch, task, prompt, allowed_tools, timeout_secs, retries).await?.0)
}

/// Como `ask_json`, mas o codigo confere a resposta (`check` devolve os erros, conferidos contra dados reais:
/// catalogo, falas) e devolve a lista ao agente NA MESMA SESSAO para ele corrigir, ate `rounds` vezes.
/// Devolve a ultima resposta valida e os erros que sobraram (o chamador decide se avisa ou falha).
pub async fn ask_json_checked<T: DeserializeOwned>(
    ch: &Channel,
    task: &str,
    prompt: &str,
    allowed_tools: &[&str],
    timeout_secs: u64,
    retries: u32,
    rounds: u32,
    check: impl Fn(&T) -> Vec<String>,
) -> Result<(T, Vec<String>)> {
    let (mut best, mut session) = ask_json_session(ch, task, prompt, allowed_tools, timeout_secs, retries).await?;
    let mut errs = check(&best);
    for round in 0..rounds {
        if errs.is_empty() || session.is_empty() {
            break;
        }
        tracing::warn!("[{}] {task}: {} erros conferidos no plano (correcao {}): {}", ch.slug, errs.len(), round + 1, errs.join(" | "));
        let msg = format!(
            "O codigo conferiu sua resposta contra o catalogo/falas e achou estes erros:
- {}

             Corrija SO isso (mantenha o resto igual) e responda APENAS com o JSON COMPLETO no mesmo formato.",
            errs.join("
- ")
        );
        let (text, sid) = ask_session(ch, task, &msg, allowed_tools, timeout_secs, Some(&session)).await?;
        if !sid.is_empty() {
            session = sid;
        }
        match extract_json(&text).and_then(|j| serde_json::from_str::<T>(j).map_err(|e| anyhow!("{e}"))) {
            Ok(v) => {
                errs = check(&v);
                best = v;
            }
            Err(e) => {
                tracing::warn!("[{}] {task}: correcao sem JSON valido ({e}); fico com a anterior", ch.slug);
                break;
            }
        }
    }
    Ok((best, errs))
}

async fn ask_json_session<T: DeserializeOwned>(
    ch: &Channel,
    task: &str,
    prompt: &str,
    allowed_tools: &[&str],
    timeout_secs: u64,
    retries: u32,
) -> Result<(T, String)> {
    let (mut text, session) = ask_session(ch, task, prompt, allowed_tools, timeout_secs, None).await?;
    let mut last_err = String::new();
    for attempt in 0..=retries {
        match extract_json(&text).and_then(|j| serde_json::from_str::<T>(j).map_err(|e| anyhow!("{e}"))) {
            Ok(v) => return Ok((v, session.clone())),
            Err(e) => {
                last_err = e.to_string();
                tracing::warn!("[{}] {task}: resposta invalida (tentativa {}): {last_err}", ch.slug, attempt + 1);
            }
        }
        if attempt == retries {
            break;
        }
        text = match extract_json(&text) {
            Ok(j) => {
                let fix = format!(
                    "O JSON abaixo falhou na validacao: {last_err}\n\nCorrija SO o necessario e responda APENAS com o objeto JSON valido, sem texto em volta.\n\n{j}"
                );
                ask(ch, task, &fix, &[], timeout_secs.min(300)).await?
            }
            // sem JSON: continua a MESMA sessao (cache quente, o trabalho ja feito fica) pedindo so o JSON;
            // antes reenviava o prompt inteiro numa sessao nova (um recreate inteiro de novo, ~US$ 0.4)
            Err(_) if !session.is_empty() => {
                let p = "A resposta anterior nao trouxe o JSON pedido. Responda AGORA APENAS com o objeto JSON valido, \
                         no formato pedido, sem texto em volta.";
                ask_session(ch, task, p, allowed_tools, timeout_secs, Some(&session)).await?.0
            }
            Err(_) => {
                let p = format!("{prompt}\n\n---\nA tentativa anterior nao devolveu JSON. Responda APENAS com o objeto JSON valido.");
                ask(ch, task, &p, allowed_tools, timeout_secs).await?
            }
        };
    }
    Err(anyhow!("claude nao devolveu JSON valido: {last_err}"))
}

/// Tira do markdown as secoes (e subsecoes) cujo titulo comeca com um dos `skip`
/// (sem diferenciar maiusculas). Usado para mandar ao agente so o que a tarefa usa.
pub fn drop_sections(text: &str, skip: &[&str]) -> String {
    let skip: Vec<String> = skip.iter().map(|k| k.to_lowercase()).collect();
    let mut out = String::with_capacity(text.len());
    let mut skip_level: Option<usize> = None;
    for line in text.lines() {
        let level = line.chars().take_while(|c| *c == '#').count();
        if level > 0 && line[level..].starts_with(' ') {
            if skip_level.is_some_and(|l| level <= l) {
                skip_level = None;
            }
            let title = line[level..].trim().to_lowercase();
            if skip_level.is_none() && skip.iter().any(|k| title.starts_with(k.as_str())) {
                skip_level = Some(level);
            }
        }
        if skip_level.is_none() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Secoes do style card que buscar imagens e catalogar clipes nao usam (texto na tela, ritmo,
/// audio, post). Ficam conceito, formato e cor/tratamento de imagem.
const STYLE_SKIP_VISUAL: [&str; 11] = [
    "estrutura temporal",
    "tipografia",
    "legenda",
    "marca d",
    "transi",
    "caption",
    "áudio",
    "audio",
    "checklist",
    "docs detalhados",
    "captions",
];

/// Style card so com a parte visual (para footage e tag_clips).
pub fn style_card_visual(ch: &Channel) -> Result<String> {
    let full = std::fs::read_to_string(ch.style_card())?;
    Ok(drop_sections(&full, &STYLE_SKIP_VISUAL))
}

/// Apaga as observacoes `once` do canal depois que foram usadas.
pub fn consume_once_notes(ch: &Channel) {
    if !ch.notes.iter().any(|n| n.once) {
        return;
    }
    let keep: Vec<_> = ch.notes.iter().filter(|n| !n.once).cloned().collect();
    let r = config::edit_channel(&ch.slug, |doc| {
        doc.remove("notes");
        if !keep.is_empty() {
            config::set_item(doc, "notes", config::to_item(&keep)?);
        }
        Ok(())
    });
    if let Err(e) = r {
        tracing::warn!("[{}] nao apaguei as observacoes usadas: {e}", ch.slug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentCfg, Global};

    /// `[tasks.review_cortes]` (nome antigo) continua valendo para a tarefa `review_clips`; o nome novo ganha.
    #[test]
    fn old_task_name_still_applies() {
        let mut g = Global::default();
        g.agents.model = "claude-opus-5-5".into();
        g.tasks.insert("review_cortes".into(), AgentPick { model: "claude-haiku-5-5".into(), effort: String::new() });
        assert_eq!(resolve_with(&g, None, "review_clips").model, "claude-haiku-5-5");
        g.tasks.insert("review_clips".into(), AgentPick { model: "claude-sonnet-5-5".into(), effort: String::new() });
        assert_eq!(resolve_with(&g, None, "review_clips").model, "claude-sonnet-5-5");
    }

    #[test]
    fn precedence() {
        let mut g = Global::default();
        g.agents.model = "claude-opus-5-5".into();
        g.agents.effort = "high".into();
        g.tasks.insert("tag_clips".into(), AgentPick { model: "claude-haiku-5-5".into(), effort: String::new() });

        let r = resolve_with(&g, None, "tag_clips");
        assert_eq!((r.model.as_str(), r.effort.as_str()), ("claude-haiku-5-5", "high"));
        assert_eq!(r.model_from, "global_task");

        let mut c = AgentCfg { model: "claude-sonnet-5-5".into(), ..Default::default() };
        let r = resolve_with(&g, Some(&c), "tag_clips");
        assert_eq!(r.model, "claude-sonnet-5-5");

        c.tasks.insert("tag_clips".into(), AgentPick { model: String::new(), effort: "low".into() });
        let r = resolve_with(&g, Some(&c), "tag_clips");
        assert_eq!((r.model.as_str(), r.effort.as_str()), ("claude-sonnet-5-5", "low"));
        assert_eq!(r.effort_from, "channel_task");

        let r = resolve_with(&g, Some(&c), "ideate");
        assert_eq!((r.model.as_str(), r.effort.as_str()), ("claude-sonnet-5-5", "high"));
    }

    #[test]
    fn cuts_sections() {
        let t = "# T\n## Conceito\nA\n## Tipografia (x)\nB\n### sub\nB\n## Cor e imagem\nC\n## Checklist\nD\n";
        assert_eq!(drop_sections(t, &STYLE_SKIP_VISUAL), "# T\n## Conceito\nA\n## Cor e imagem\nC\n");
    }

    #[test]
    fn envelope() {
        let s = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":5120,"num_turns":3,"result":"{\"a\":1}","session_id":"abc","total_cost_usd":0.21,"usage":{"input_tokens":12,"cache_creation_input_tokens":15000,"cache_read_input_tokens":42000,"output_tokens":800}}"#;
        let e = parse_envelope(s).unwrap();
        assert_eq!(e.num_turns, 3);
        assert_eq!(e.usage.cache_read_input_tokens, 42000);
        assert_eq!(e.result, "{\"a\":1}");
    }
}
