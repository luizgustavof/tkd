//! Avaliador local (edit-evaluator v4, llama.cpp na GPU) usado na review.
//! Roda `evaluator/eval_video.py`, que analisa o mp4 (cortes, falas, audio, defeitos medidos),
//! sobe o llama-server so durante a avaliacao e devolve notas, tags e problemas com o segundo.
//! A v4 foi treinada com edits de anime: so avalia video feito de footage (ver `in_domain`).

use crate::proc::TreeExt;
use crate::spec::Spec;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Faixa em que a nota da v4 nao basta (erro medio ~0.6 em volta do corte 7): o Claude confirma.
pub const DOUBT_LOW: f64 = 6.4;
pub const DOUBT_HIGH: f64 = 7.6;
/// 1 em cada N aprovacoes da v4 o Claude confere mesmo assim (auditoria).
pub const AUDIT_EVERY: u64 = 5;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Problem {
    #[serde(default)]
    pub t: Option<Vec<f64>>,
    #[serde(default)]
    #[serde(rename = "tipo")]
    pub ptype: String,
    #[serde(default)]
    #[serde(rename = "gravidade")]
    pub severity: String,
    #[serde(default)]
    #[serde(rename = "o_que")]
    pub what_text: String,
    #[serde(default)]
    #[serde(rename = "consertar")]
    pub repair: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Strong {
    #[serde(default)]
    pub t: Option<Vec<f64>>,
    #[serde(default)]
    #[serde(rename = "o_que")]
    pub what_text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LocalEval {
    pub model: String,
    #[serde(default)]
    pub overall: Option<f64>,
    #[serde(default)]
    pub scores: serde_json::Value,
    #[serde(default)]
    pub tags: serde_json::Value,
    #[serde(default)]
    #[serde(rename = "resumo")]
    pub recap: String,
    #[serde(default)]
    pub problemas: Vec<Problem>,
    #[serde(default)]
    #[serde(rename = "pontos_fortes")]
    pub strengths: Vec<Strong>,
    /// defeitos que as regras medidas acharam (congelado, audio estourado...)
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub rule_problems: Vec<Problem>,
    #[serde(default)]
    pub seconds: f64,
}

impl Problem {
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
        let mut out: Vec<&Problem> = self.problemas.iter().collect();
        for r in &self.rule_problems {
            if !out.iter().any(|p| p.ptype == r.ptype) {
                out.push(r);
            }
        }
        out
    }
    /// As regras acharam defeito grave (o modelo pode ter deixado passar).
    pub fn rules_grave(&self) -> bool {
        self.rule_problems.iter().any(|p| p.severity == "alta")
    }
    /// Texto pro prompt do Claude.
    pub fn brief(&self) -> String {
        let mut s = format!(
            "Nota geral {:.1} (modelo {}). Notas: {}\nResumo: {}\n",
            self.score(),
            self.model,
            self.scores,
            self.recap
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

fn evaluator_python() -> PathBuf {
    let v = crate::config::root().join("evaluator").join(".venv");
    if cfg!(windows) { v.join("Scripts").join("python.exe") } else { v.join("bin").join("python") }
}

fn gguf_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join("models-gguf").join("edit-evaluator-v4"))
}

/// Tudo instalado? (venv do avaliador, venv do rea-video, GGUF da v4 e llama-server no PATH)
pub fn available() -> Result<(), String> {
    let root = crate::config::root();
    if !evaluator_python().exists() {
        return Err("evaluator/.venv nao existe".into());
    }
    if !crate::recreate::venv_bin("python").exists() {
        return Err("venv do rea-video nao existe".into());
    }
    let Some(g) = gguf_dir() else { return Err("pasta do usuario desconhecida".into()) };
    if !g.join("edit-evaluator-v4-Q8_0.gguf").exists() || !g.join("mmproj-F16.gguf").exists() {
        return Err(format!("GGUF da v4 nao esta em {}", g.display()));
    }
    if !root.join("evaluator").join("eval_video.py").exists() {
        return Err("evaluator/eval_video.py nao existe".into());
    }
    let found = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(if cfg!(windows) { "llama-server.exe" } else { "llama-server" }).exists()))
        .unwrap_or(false);
    if !found {
        return Err("llama-server nao esta no PATH".into());
    }
    Ok(())
}

/// O video e um edit (footage na maior parte do tempo)? A v4 so conhece edits.
pub fn in_domain(spec: &Spec) -> bool {
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
    footage / total >= 0.5
}

/// Sorteio da auditoria, estavel por video (repetir a review nao muda o resultado).
pub fn audit_pick(video_id: &str) -> bool {
    let h = video_id.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
    h % AUDIT_EVERY == 0
}

/// Avaliacoes juntas no processo (todos os canais). Cada uma pede ~5 GB de RAM e ~5 nucleos na analise;
/// o modelo na GPU ja e um por vez (trava em arquivo no eval_video.py).
pub const MAX_PARALLEL: usize = 2;

fn eval_slots() -> &'static tokio::sync::Semaphore {
    static S: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    S.get_or_init(|| tokio::sync::Semaphore::new(MAX_PARALLEL))
}

pub async fn evaluate(mp4: &Path, caption: &str) -> Result<LocalEval> {
    if eval_slots().available_permits() == 0 {
        tracing::info!("avaliador local: {MAX_PARALLEL} avaliacoes rodando, {} na fila", mp4.display());
    }
    let _slot = eval_slots().acquire().await.context("fila do avaliador local fechada")?;
    let root = crate::config::root();
    let out = tokio::process::Command::new(evaluator_python())
        .args(["-I", "-X", "utf8"])
        .arg(root.join("evaluator").join("eval_video.py"))
        .arg(mp4)
        .arg("--caption")
        .arg(caption)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .current_dir(&root)
        // teto: llama-server ou decompose travado segurava 1 das 2 vagas do avaliador para sempre
        .tree_output_timeout(std::time::Duration::from_secs(15 * 60))
        .await
        .context("rodando o avaliador local")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("avaliador local falhou: {}", err.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let ev: LocalEval = serde_json::from_str(line).context("resposta do avaliador local")?;
    anyhow::ensure!(ev.overall.is_some(), "avaliador local sem nota");
    Ok(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saida do avaliador v4: chaves em portugues (o modelo foi treinado assim).
    #[test]
    fn reads_v4_output_keys() {
        let j = r#"{"model":"v4","overall":7.0,"resumo":"ok","problemas":[{"t":[1.0,2.0],"tipo":"hook_fraco","gravidade":"alta","o_que":"x","consertar":"y"}],"pontos_fortes":[{"o_que":"z"}]}"#;
        let ev: LocalEval = serde_json::from_str(j).unwrap();
        assert_eq!(ev.recap, "ok");
        let p = &ev.problemas[0];
        assert_eq!((p.ptype.as_str(), p.severity.as_str(), p.what_text.as_str(), p.repair.as_str()), ("hook_fraco", "alta", "x", "y"));
        assert_eq!(ev.strengths[0].what_text, "z");
    }

    #[test]
    fn audit_is_stable_and_rare() {
        let ids: Vec<String> = (0..1000).map(|i| format!("canal-{i}")).collect();
        let n = ids.iter().filter(|i| audit_pick(i)).count();
        assert!((120..=280).contains(&n), "{n}");
        assert_eq!(audit_pick("abc"), audit_pick("abc"));
    }

    #[test]
    fn merges_problems_without_repeating_kind() {
        let p = |ptype: &str, g: &str| Problem { ptype: ptype.into(), severity: g.into(), ..Default::default() };
        let ev = LocalEval {
            problemas: vec![p("hook_fraco", "media")],
            rule_problems: vec![p("hook_fraco", "alta"), p("audio_estourado", "alta")],
            ..Default::default()
        };
        assert_eq!(ev.all_problems().len(), 2);
        assert!(ev.rules_grave());
    }
}
