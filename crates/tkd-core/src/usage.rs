//! Consumo dos agentes.
//!
//! Chamadas novas sao gravadas por `agent::ask` em `agent_calls`. O historico anterior vem
//! dos `.jsonl` que o Claude Code guarda em ~/.claude/projects/<pasta do projeto>/: a tarefa
//! e reconhecida pela primeira linha do template (agents/*.md) na primeira mensagem.

use crate::state::{AgentCall, Store, UsageRow};
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Assinatura (inicio da 1a linha do template) -> tarefa.
const SIGNATURES: [(&str, &str); 6] = [
    ("study", "Voce e o diretor de arte do canal"),
    ("ideate", "Voce e o roteirista/editor do canal"),
    ("review", "Voce e o critico de qualidade do canal"),
    ("recreate", "Voce e o editor de recriacao do canal"),
    ("footage", "Voce e o pesquisador de imagens do canal"),
    ("tag_clips", "Voce esta catalogando o banco de clipes"),
];

/// Canal usado para as conversas interativas (fora dos canais).
pub const INTERACTIVE: &str = "(conversas)";

/// Pasta do Claude Code para um caminho de projeto: todo caractere fora de [A-Za-z0-9-] vira '-'.
pub fn claude_project_dir(root: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let s: String = root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    Some(PathBuf::from(home).join(".claude").join("projects").join(s))
}

/// Uma sessao (.jsonl) resumida.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub session: String,
    pub task: String,
    pub channel: String,
    pub model: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub turns: i64,
    pub first_at: String,
    pub last_at: String,
}

fn text_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn channel_in(first: &str) -> Option<String> {
    for pat in ["canal TikTok `", "canal `"] {
        if let Some(i) = first.find(pat) {
            let rest = &first[i + pat.len()..];
            if let Some(j) = rest.find('`') {
                return Some(rest[..j].to_string());
            }
        }
    }
    None
}

/// Le um .jsonl do Claude Code (tolerante a linhas quebradas).
pub fn parse_session(text: &str, fallback_id: &str) -> Session {
    let mut s = Session { session: fallback_id.to_string(), ..Default::default() };
    let mut first: Option<String> = None;
    let mut seen: HashSet<String> = HashSet::new();
    let mut models: HashMap<String, i64> = HashMap::new();
    for line in text.lines() {
        let Ok(o) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if let Some(id) = o["sessionId"].as_str() {
            s.session = id.to_string();
        }
        if let Some(ts) = o["timestamp"].as_str() {
            if s.first_at.is_empty() {
                s.first_at = ts.to_string();
            }
            s.last_at = ts.to_string();
        }
        match o["type"].as_str() {
            Some("user") if first.is_none() => {
                let t = text_of(&o["message"]["content"]);
                if !t.trim().is_empty() {
                    first = Some(t);
                }
            }
            Some("assistant") => {
                let m = &o["message"];
                let id = m["id"].as_str().unwrap_or_default().to_string();
                // a mesma mensagem aparece uma vez por bloco de conteudo
                if !id.is_empty() && !seen.insert(id) {
                    continue;
                }
                let u = &m["usage"];
                s.input += u["input_tokens"].as_i64().unwrap_or(0);
                s.output += u["output_tokens"].as_i64().unwrap_or(0);
                s.cache_read += u["cache_read_input_tokens"].as_i64().unwrap_or(0);
                s.cache_write += u["cache_creation_input_tokens"].as_i64().unwrap_or(0);
                s.turns += 1;
                if let Some(md) = m["model"].as_str() {
                    *models.entry(md.to_string()).or_default() += 1;
                }
            }
            _ => {}
        }
    }
    let first = first.unwrap_or_default();
    match SIGNATURES.iter().find(|(_, sig)| first.contains(sig)) {
        Some((task, _)) => {
            s.task = task.to_string();
            s.channel = channel_in(&first).unwrap_or_else(|| "?".into());
        }
        None => {
            s.task = "interativo".into();
            s.channel = INTERACTIVE.into();
        }
    }
    s.model = models.into_iter().max_by_key(|(_, n)| *n).map(|(m, _)| m).unwrap_or_default();
    s
}

fn ms_between(a: &str, b: &str) -> i64 {
    match (chrono::DateTime::parse_from_rfc3339(a), chrono::DateTime::parse_from_rfc3339(b)) {
        (Ok(a), Ok(b)) => (b - a).num_milliseconds().max(0),
        _ => 0,
    }
}

/// Importa (ou atualiza) as sessoes do ~/.claude que mudaram desde a ultima importacao.
/// Devolve quantas sessoes foram lidas.
///
/// Por arquivo (tamanho + mtime em ns): a marca unica em segundos pulava para sempre um arquivo salvo no mesmo
/// segundo do mais novo, e qualquer mudanca relia tudo o que tinha mtime maior. Sessoes do proprio tkd
/// (`claude -p`, ja gravadas com os numeros oficiais) nem sao abertas: eram a maioria dos ~400 MB relidos.
pub fn import_history(store: &Store) -> Result<usize> {
    let Some(dir) = claude_project_dir(&crate::config::root()) else { return Ok(0) };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(0) };
    store.db().execute("CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)", [])?;
    let mut seen: HashMap<String, String> = store
        .db()
        .query_row("SELECT v FROM kv WHERE k = 'usage_import_files'", [], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    let tkd_sessions: HashSet<String> = {
        let db = store.db();
        let mut st = db.prepare("SELECT session FROM agent_calls WHERE source = 'tkd' AND session != ''")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut n = 0;
    let mut changed = false;
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if tkd_sessions.contains(&stem) {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mark = format!("{}:{mtime}", meta.len());
        if seen.get(&stem) == Some(&mark) {
            continue;
        }
        seen.insert(stem.clone(), mark);
        changed = true;
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let s = parse_session(&text, &stem);
        if s.turns == 0 {
            continue;
        }
        upsert(store, &s)?;
        n += 1;
    }
    if changed {
        store.db().execute(
            "INSERT INTO kv (k, v) VALUES ('usage_import_files', ?1) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![serde_json::to_string(&seen)?],
        )?;
    }
    Ok(n)
}

fn upsert(store: &Store, s: &Session) -> Result<()> {
    let c = AgentCall {
        channel: s.channel.clone(),
        task: s.task.clone(),
        model: s.model.clone(),
        effort: String::new(),
        input: s.input,
        output: s.output,
        cache_read: s.cache_read,
        cache_write: s.cache_write,
        cost: 0.0,
        turns: s.turns,
        ms: ms_between(&s.first_at, &s.last_at),
        ok: true,
        source: "import".into(),
        session: s.session.clone(),
        at: s.first_at.clone(),
    };
    // chamada feita pelo tkd (source = tkd) ja tem os numeros oficiais: nao sobrescreve
    store.db().execute(
        "INSERT INTO agent_calls (channel, task, model, effort, input, output, cache_read, cache_write, cost, turns, ms, ok, source, session, at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(session) WHERE session != '' DO UPDATE SET
           input = excluded.input, output = excluded.output, cache_read = excluded.cache_read,
           cache_write = excluded.cache_write, turns = excluded.turns, ms = excluded.ms, model = excluded.model
         WHERE agent_calls.source = 'import'",
        params![
            c.channel, c.task, c.model, c.effort, c.input, c.output, c.cache_read, c.cache_write, c.cost, c.turns,
            c.ms, c.ok as i32, c.source, c.session, c.at
        ],
    )?;
    Ok(())
}

/// Quando o `claude -p` passou a rodar isolado (sem hooks/MCP/skills/memoria, ferramentas so da
/// tarefa, teto de turnos). Antes x depois na tela de Consumo.
pub const TOKEN_FIX_AT: &str = "2026-11-16T23:40:00+00:00";

#[derive(Debug, Clone, Serialize, Default)]
pub struct TaskCompare {
    pub task: String,
    pub before_calls: i64,
    pub before_avg: i64,
    pub after_calls: i64,
    pub after_avg: i64,
}

/// Tokens medios por chamada, por tarefa, antes e depois de `TOKEN_FIX_AT`.
pub fn before_after(store: &Store) -> Result<Vec<TaskCompare>> {
    let avg = |tot: i64, n: i64| if n > 0 { tot / n } else { 0 };
    Ok(store
        .usage_split(TOKEN_FIX_AT)?
        .into_iter()
        .map(|(task, bc, bt, ac, at)| TaskCompare { task, before_calls: bc, before_avg: avg(bt, bc), after_calls: ac, after_avg: avg(at, ac) })
        .collect())
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Summary {
    pub since: String,
    pub total: UsageRow,
    pub by_channel: Vec<UsageRow>,
    pub by_task: Vec<UsageRow>,
    pub by_pair: Vec<UsageRow>,
    pub by_model: Vec<UsageRow>,
    /// (dia, tokens novos, leitura de cache)
    pub daily: Vec<(String, i64, i64)>,
}

/// Resumo do consumo dos ultimos `days` dias (0 = tudo).
pub fn summary(store: &Store, days: i64) -> Result<Summary> {
    let since = if days <= 0 { "0000".to_string() } else { crate::state::days_ago(days) };
    let by_channel = store.usage_by("channel", &since)?;
    let mut total = UsageRow { channel: "total".into(), ..Default::default() };
    for r in &by_channel {
        total.calls += r.calls;
        total.input += r.input;
        total.output += r.output;
        total.cache_read += r.cache_read;
        total.cache_write += r.cache_write;
        total.cost += r.cost;
        total.turns += r.turns;
        total.ms += r.ms;
    }
    Ok(Summary {
        since: since.clone(),
        total,
        by_channel,
        by_task: store.usage_by("task", &since)?,
        by_pair: store.usage_by("pair", &since)?,
        by_model: store.usage_by("model", &since)?,
        daily: store.usage_daily(&since)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_session() {
        let j = r#"{"type":"user","sessionId":"s1","timestamp":"2026-11-14T10:00:00Z","message":{"content":"Voce e o roteirista/editor do canal TikTok `meu-canal`.\n..."}}
{"type":"assistant","timestamp":"2026-11-14T10:00:05Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":3,"output_tokens":100,"cache_read_input_tokens":1000,"cache_creation_input_tokens":15000}}}
{"type":"assistant","timestamp":"2026-11-14T10:00:05Z","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":3,"output_tokens":100,"cache_read_input_tokens":1000,"cache_creation_input_tokens":15000}}}
{"type":"assistant","timestamp":"2026-11-14T10:00:09Z","message":{"id":"m2","model":"claude-opus-5-5","usage":{"input_tokens":1,"output_tokens":50,"cache_read_input_tokens":16000,"cache_creation_input_tokens":0}}}
quebrada"#;
        let s = parse_session(j, "x");
        assert_eq!(s.task, "ideate");
        assert_eq!(s.channel, "meu-canal");
        assert_eq!(s.turns, 2);
        assert_eq!(s.cache_read, 17000);
        assert_eq!(s.cache_write, 15000);
        assert_eq!(s.session, "s1");
        assert_eq!(ms_between(&s.first_at, &s.last_at), 9000);
    }

    #[test]
    fn project_dir_name() {
        let p = claude_project_dir(Path::new(r"C:\Users\dev\Área de Trabalho\tkd")).unwrap();
        assert!(p.ends_with("C--Users-dev--rea-de-Trabalho-tkd"));
    }
}
