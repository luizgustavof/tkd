//! Ponte para um avaliador local opcional, usado na review dos edits.
//!
//! O tkd nao traz modelo nenhum: quem quiser liga o proprio no `tkd.toml`, secao `[evaluator]`, com o
//! comando que avalia um mp4 e imprime um JSON (protocolo em `docs/EVALUATOR.md`). Sem a secao, a review
//! segue so com o QA medido e o Claude. Com ela:
//! - nota clara (abaixo de `doubt_low` ou acima de `doubt_high`) decide sozinha;
//! - nota na faixa duvidosa, ou defeito grave medido, chama o Claude com a avaliacao do modelo junto;
//! - 1 em cada `audit_every` aprovacoes o Claude confere mesmo assim (auditoria).

use crate::proc::TreeExt;
use crate::spec::Spec;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `[evaluator]` do tkd.toml.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct EvaluatorCfg {
    /// Nome nos logs e na tela (ex.: "meu-modelo-v2").
    pub name: String,
    /// Programa e argumentos. `{video}`, `{caption}` e `{root}` sao trocados; caminho relativo vale a
    /// partir da pasta do projeto. Vazio = sem avaliador.
    pub command: Vec<String>,
    /// So avalia video com pelo menos essa fracao de footage (0 = qualquer edit).
    pub min_footage: f64,
    /// Nota <= doubt_low reprova sem o Claude; nota >= doubt_high aprova sem o Claude.
    pub doubt_low: f64,
    pub doubt_high: f64,
    /// 1 em cada N aprovacoes o Claude confere (0 = nunca).
    pub audit_every: u64,
    /// Avaliacoes ao mesmo tempo (vale na proxima abertura do app).
    pub parallel: usize,
    /// Teto de cada avaliacao, em minutos.
    pub timeout_min: u64,
}

impl Default for EvaluatorCfg {
    fn default() -> Self {
        Self {
            name: "local".into(),
            command: vec![],
            min_footage: 0.5,
            doubt_low: 6.4,
            doubt_high: 7.6,
            audit_every: 5,
            parallel: 2,
            timeout_min: 15,
        }
    }
}

impl EvaluatorCfg {
    pub fn enabled(&self) -> bool {
        self.command.first().is_some_and(|p| !p.trim().is_empty())
    }
}

/// Le o `[evaluator]` do tkd.toml (ausente ou invalido = desligado).
pub fn config() -> EvaluatorCfg {
    let path = crate::config::global_path();
    let Some(v) = std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str::<toml::Value>(&t).ok()) else {
        return EvaluatorCfg::default();
    };
    match v.get("evaluator").cloned().map(|e| e.try_into::<EvaluatorCfg>()) {
        Some(Ok(c)) => c,
        Some(Err(e)) => {
            tracing::error!("[evaluator] invalido no tkd.toml: {e}");
            EvaluatorCfg::default()
        }
        None => EvaluatorCfg::default(),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Problem {
    /// [inicio, fim] em segundos
    #[serde(default)]
    pub t: Option<Vec<f64>>,
    #[serde(default, rename = "type", alias = "tipo")]
    pub ptype: String,
    /// high | medium | low (aceita alta | media | baixa)
    #[serde(default, alias = "gravidade")]
    pub severity: String,
    #[serde(default, rename = "what", alias = "o_que")]
    pub what_text: String,
    #[serde(default, rename = "fix", alias = "consertar")]
    pub repair: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Strong {
    #[serde(default)]
    pub t: Option<Vec<f64>>,
    #[serde(default, rename = "what", alias = "o_que")]
    pub what_text: String,
}

/// Resposta do avaliador (ultima linha do stdout que comeca com `{`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LocalEval {
    #[serde(default)]
    pub model: String,
    /// nota geral de 0 a 10 (obrigatoria)
    #[serde(default)]
    pub overall: Option<f64>,
    #[serde(default)]
    pub scores: serde_json::Value,
    #[serde(default)]
    pub tags: serde_json::Value,
    #[serde(default, alias = "resumo")]
    pub summary: String,
    #[serde(default, alias = "problemas")]
    pub problems: Vec<Problem>,
    #[serde(default, alias = "pontos_fortes")]
    pub strengths: Vec<Strong>,
    /// defeitos medidos por regras do proprio avaliador (congelado, audio estourado...)
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub rule_problems: Vec<Problem>,
    #[serde(default)]
    pub seconds: f64,
}

impl Problem {
    pub fn is_high(&self) -> bool {
        matches!(self.severity.to_lowercase().as_str(), "high" | "alta" | "grave")
    }
    pub fn line(&self) -> String {
        let t = match self.t.as_deref() {
            Some([a, b, ..]) => format!("[{a:.1}-{b:.1}s] "),
            _ => String::new(),
        };
        let fix = if self.repair.is_empty() { String::new() } else { format!(" -> {}", self.repair) };
        format!("{t}{}{fix}", self.what_text)
    }
}

impl LocalEval {
    pub fn score(&self) -> f64 {
        self.overall.unwrap_or(0.0)
    }
    /// Problemas do modelo + os medidos pelas regras (sem repetir o mesmo tipo).
    pub fn all_problems(&self) -> Vec<&Problem> {
        let mut out: Vec<&Problem> = self.problems.iter().collect();
        for r in &self.rule_problems {
            if !out.iter().any(|p| p.ptype == r.ptype) {
                out.push(r);
            }
        }
        out
    }
    /// As regras acharam defeito grave (o modelo pode ter deixado passar).
    pub fn rules_grave(&self) -> bool {
        self.rule_problems.iter().any(Problem::is_high)
    }
    /// Texto pro prompt do Claude.
    pub fn brief(&self) -> String {
        let mut s = format!(
            "Nota geral {:.1} (modelo {}). Notas: {}\nResumo: {}\n",
            self.score(),
            self.model,
            self.scores,
            self.summary
        );
        let probs = self.all_problems();
        if probs.is_empty() {
            s.push_str("Problemas: nenhum\n");
        } else {
            s.push_str("Problemas (segundo exato):\n");
            for p in probs {
                s.push_str(&format!("- ({}) {}\n", if p.severity.is_empty() { "?" } else { &p.severity }, p.line()));
            }
        }
        if !self.rules.is_empty() {
            s.push_str(&format!("Defeitos medidos pelas regras: {}\n", self.rules.join(", ")));
        }
        s
    }
}

/// Caminho do programa: relativo a pasta do projeto se tiver barra, senao procurado no PATH.
fn resolve_program(p: &str, root: &Path) -> Option<PathBuf> {
    let has_sep = p.contains('/') || p.contains('\\');
    if has_sep || Path::new(p).is_absolute() {
        let full = if Path::new(p).is_absolute() { PathBuf::from(p) } else { root.join(p) };
        return full.exists().then_some(full);
    }
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).find_map(|d| exts.iter().map(|e| d.join(format!("{p}{e}"))).find(|f| f.is_file()))
    })
}

/// O avaliador esta configurado e o programa existe?
pub fn available(cfg: &EvaluatorCfg) -> Result<PathBuf, String> {
    if !cfg.enabled() {
        return Err("nenhum avaliador configurado ([evaluator] no tkd.toml)".into());
    }
    let prog = &cfg.command[0];
    resolve_program(prog, &crate::config::root()).ok_or_else(|| format!("programa do avaliador nao encontrado: {prog}"))
}

/// O video tem footage suficiente para o avaliador (`min_footage`)?
pub fn in_domain(spec: &Spec, cfg: &EvaluatorCfg) -> bool {
    let total: f64 = spec.scenes.iter().map(|s| s.duration_s).sum();
    if total <= 0.0 {
        return false;
    }
    let footage: f64 = spec
        .scenes
        .iter()
        .filter(|s| {
            s.media.as_deref().is_some_and(|m| {
                let m = m.to_lowercase();
                [".mp4", ".mov", ".webm", ".mkv"].iter().any(|e| m.ends_with(e))
            })
        })
        .map(|s| s.duration_s)
        .sum();
    footage / total >= cfg.min_footage
}

/// Sorteio da auditoria, estavel por video (repetir a review nao muda o resultado).
pub fn audit_pick(video_id: &str, every: u64) -> bool {
    if every == 0 {
        return false;
    }
    let h = video_id.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
    h % every == 0
}

fn eval_slots(n: usize) -> &'static tokio::sync::Semaphore {
    static S: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    S.get_or_init(|| tokio::sync::Semaphore::new(n.clamp(1, 8)))
}

/// Argumentos com `{video}`, `{caption}` e `{root}` trocados.
fn expand(args: &[String], mp4: &Path, caption: &str, root: &Path) -> Vec<String> {
    args.iter()
        .map(|a| {
            a.replace("{video}", &mp4.to_string_lossy())
                .replace("{caption}", caption)
                .replace("{root}", &root.to_string_lossy())
        })
        .collect()
}

/// Le a resposta: a ultima linha do stdout que comeca com `{`.
pub fn parse_output(stdout: &str) -> Result<LocalEval> {
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let ev: LocalEval = serde_json::from_str(line).context("resposta do avaliador (JSON numa linha)")?;
    anyhow::ensure!(ev.overall.is_some_and(|n| (0.0..=10.0).contains(&n)), "avaliador sem nota `overall` de 0 a 10");
    Ok(ev)
}

pub async fn evaluate(cfg: &EvaluatorCfg, mp4: &Path, caption: &str) -> Result<LocalEval> {
    evaluate_in(cfg, mp4, caption, &crate::config::root()).await
}

/// `evaluate` com a pasta do projeto explicita (os testes nao dependem da raiz global).
async fn evaluate_in(cfg: &EvaluatorCfg, mp4: &Path, caption: &str, root: &Path) -> Result<LocalEval> {
    if !cfg.enabled() {
        anyhow::bail!("nenhum avaliador configurado ([evaluator] no tkd.toml)");
    }
    let prog = resolve_program(&cfg.command[0], root)
        .ok_or_else(|| anyhow::anyhow!("programa do avaliador nao encontrado: {}", cfg.command[0]))?;
    let slots = eval_slots(cfg.parallel);
    if slots.available_permits() == 0 {
        tracing::info!("avaliador {}: fila cheia, {} esperando", cfg.name, mp4.display());
    }
    let _slot = slots.acquire().await.context("fila do avaliador fechada")?;
    let started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(prog);
    cmd.args(expand(&cfg.command[1..], mp4, caption, root))
        .env("TKD_EVAL_VIDEO", mp4)
        .env("TKD_EVAL_CAPTION", caption)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::null());
    // caminhos relativos do `command` valem a partir da pasta do projeto (sem ela, nunca herdar uma pasta
    // atual que pode ter sumido: o Windows recusa criar o processo, erro 267)
    let cwd = if root.is_dir() { root.to_path_buf() } else { std::env::temp_dir() };
    cmd.current_dir(&cwd);
    let out = cmd
        // teto: um avaliador travado segurava uma vaga da fila para sempre
        .tree_output_timeout(std::time::Duration::from_secs(cfg.timeout_min.max(1) * 60))
        .await
        .with_context(|| format!("rodando o avaliador {} ({} em {})", cfg.name, cfg.command[0], cwd.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("avaliador {} falhou: {}", cfg.name, err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
    }
    let mut ev = parse_output(&String::from_utf8_lossy(&out.stdout))?;
    if ev.model.is_empty() {
        ev.model = cfg.name.clone();
    }
    if ev.seconds <= 0.0 {
        ev.seconds = started.elapsed().as_secs_f64();
    }
    Ok(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_english_keys() {
        let j = r#"{"overall":7.0,"summary":"ok","problems":[{"t":[1.0,2.0],"type":"weak_hook","severity":"high","what":"x","fix":"y"}],"strengths":[{"what":"z"}]}"#;
        let ev = parse_output(j).unwrap();
        assert_eq!(ev.summary, "ok");
        let p = &ev.problems[0];
        assert_eq!((p.ptype.as_str(), p.what_text.as_str(), p.repair.as_str()), ("weak_hook", "x", "y"));
        assert!(p.is_high());
        assert_eq!(ev.strengths[0].what_text, "z");
    }

    /// Chaves em portugues (avaliadores treinados assim) continuam valendo.
    #[test]
    fn reads_portuguese_keys() {
        let j = "log qualquer\n{\"model\":\"v4\",\"overall\":7.0,\"resumo\":\"ok\",\"problemas\":[{\"tipo\":\"hook_fraco\",\"gravidade\":\"alta\",\"o_que\":\"x\",\"consertar\":\"y\"}],\"pontos_fortes\":[{\"o_que\":\"z\"}]}";
        let ev = parse_output(j).unwrap();
        assert_eq!((ev.model.as_str(), ev.summary.as_str()), ("v4", "ok"));
        assert_eq!(ev.problems[0].ptype, "hook_fraco");
        assert!(ev.problems[0].is_high());
        assert_eq!(ev.strengths[0].what_text, "z");
    }

    #[test]
    fn rejects_missing_or_bad_score() {
        assert!(parse_output(r#"{"summary":"sem nota"}"#).is_err());
        assert!(parse_output(r#"{"overall":42}"#).is_err());
        assert!(parse_output("nada de json").is_err());
    }

    #[test]
    fn config_defaults_and_toml() {
        assert!(!EvaluatorCfg::default().enabled());
        let v: toml::Value = toml::from_str(
            "[evaluator]\nname = \"meu\"\ncommand = [\"python\", \"eval.py\", \"{video}\"]\ndoubt_low = 6.0\n",
        )
        .unwrap();
        let c: EvaluatorCfg = v.get("evaluator").cloned().unwrap().try_into().unwrap();
        assert!(c.enabled());
        assert_eq!((c.name.as_str(), c.doubt_low, c.doubt_high), ("meu", 6.0, 7.6));
    }

    #[test]
    fn expands_placeholders() {
        let a = expand(&["{video}".into(), "--caption".into(), "{caption}".into(), "{root}/x".into()], Path::new("v.mp4"), "oi", Path::new("R"));
        assert_eq!(a, vec!["v.mp4", "--caption", "oi", "R/x"]);
    }

    #[test]
    fn audit_is_stable_and_rare() {
        let ids: Vec<String> = (0..1000).map(|i| format!("canal-{i}")).collect();
        let n = ids.iter().filter(|i| audit_pick(i, 5)).count();
        assert!((120..=280).contains(&n), "{n}");
        assert_eq!(audit_pick("abc", 5), audit_pick("abc", 5));
        assert!(!ids.iter().any(|i| audit_pick(i, 0)));
    }

    fn py(code: &str) -> EvaluatorCfg {
        EvaluatorCfg { name: "teste".into(), command: vec!["python".into(), "-c".into(), code.into(), "{video}".into()], timeout_min: 1, ..Default::default() }
    }

    /// Roda um comando de verdade: argumentos trocados, ultima linha JSON lida, nome do modelo herdado.
    #[tokio::test]
    async fn runs_a_real_command() {
        if resolve_program("python", Path::new(".")).is_none() {
            eprintln!("sem python no PATH: pulado");
            return;
        }
        let root = std::env::temp_dir();
        let ok = py("import sys, json; print('log'); print(json.dumps({'overall': 7.5, 'summary': sys.argv[1]}))");
        let ev = evaluate_in(&ok, Path::new("video.mp4"), "oi", &root).await.unwrap();
        assert_eq!((ev.score(), ev.summary.as_str(), ev.model.as_str()), (7.5, "video.mp4", "teste"));
        let fails = py("import sys; print('quebrou', file=sys.stderr); sys.exit(3)");
        let e = format!("{:#}", evaluate_in(&fails, Path::new("v.mp4"), "", &root).await.unwrap_err());
        assert!(e.contains("quebrou"), "{e}");
    }

    #[test]
    fn merges_problems_without_repeating_kind() {
        let p = |ptype: &str, g: &str| Problem { ptype: ptype.into(), severity: g.into(), ..Default::default() };
        let ev = LocalEval {
            problems: vec![p("hook_fraco", "media")],
            rule_problems: vec![p("hook_fraco", "alta"), p("audio_estourado", "high")],
            ..Default::default()
        };
        assert_eq!(ev.all_problems().len(), 2);
        assert!(ev.rules_grave());
    }
}
