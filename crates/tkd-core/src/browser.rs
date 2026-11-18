use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Um navegador por vez (regra anti-bot: nunca dois Edge automatizados juntos).
fn lock() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(1))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Holder {
    pub channel: String,
    pub script: String,
    pub since: String,
}

static HOLDER: Mutex<Option<Holder>> = Mutex::new(None);
static WAITING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Quem esta com o navegador agora e quem esta na fila.
pub fn status() -> (Option<Holder>, Vec<String>) {
    (
        HOLDER.lock().ok().and_then(|g| g.clone()),
        WAITING.lock().map(|g| g.clone()).unwrap_or_default(),
    )
}

/// Roda browser/<script>.mjs com o payload JSON e le a ultima linha do stdout.
///
/// Linhas `{"evt":...}` no stderr viram eventos ao vivo (acao, progresso, url).
pub async fn run<T: DeserializeOwned>(script: &str, payload: &Value) -> Result<T> {
    let root = crate::config::root();
    let script_path = root.join("browser").join(script);
    anyhow::ensure!(script_path.exists(), "script {} nao existe", script_path.display());
    let channel = payload["profile"].as_str().unwrap_or("default").to_string();

    // sai da fila mesmo se a task for cancelada enquanto espera (senao a tela mostra "espera" para sempre)
    struct Waiting(String);
    impl Drop for Waiting {
        fn drop(&mut self) {
            if let Ok(mut w) = WAITING.lock() {
                if let Some(i) = w.iter().position(|c| *c == self.0) {
                    w.remove(i);
                }
            }
        }
    }
    if let Ok(mut w) = WAITING.lock() {
        w.push(channel.clone());
    }
    let waiting = Waiting(channel.clone());
    let permit = lock().acquire().await;
    drop(waiting);
    let _permit = permit.map_err(|_| anyhow!("trava do navegador fechada"))?;
    // a trava acima e so deste processo: app e CLI juntos abriam dois Edge (regra anti-bot) e brigavam pelo
    // mesmo perfil. Esta vale entre processos e cai sozinha se o dono morrer.
    let lock_path = root.join(".profiles").join(".browser.lock");
    let _cross = crate::proc::ExclusiveFile::acquire(&lock_path, || {
        tracing::info!("navegador em uso por outro processo do tkd (app ou CLI): esperando");
    })
    .await;
    if let Ok(mut h) = HOLDER.lock() {
        *h = Some(Holder { channel: channel.clone(), script: script.into(), since: crate::state::now() });
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            if let Ok(mut h) = HOLDER.lock() {
                *h = None;
            }
        }
    }
    let _release = Release;

    let mut cmd = Command::new("node");
    cmd.arg(&script_path).arg("--json").arg(payload.to_string()).current_dir(root.join("browser"));
    // teto por script: um upload travado segurava o unico navegador de todos os canais para sempre
    let limit = payload["timeoutMin"].as_u64().map(|m| (m + 2) * 60).unwrap_or(DEFAULT_LIMIT_SECS);
    let limit = crate::chaos::limit(std::time::Duration::from_secs(limit)).as_secs().max(1);
    let label = format!("Abrindo o Edge ({})", script.trim_end_matches(".mjs"));
    tokio::time::timeout(std::time::Duration::from_secs(limit), exec(&channel, script, &label, cmd))
        .await
        .map_err(|_| anyhow!("{script} passou de {}: navegador fechado", human_secs(limit)))?
}

fn human_secs(s: u64) -> String {
    if s >= 60 {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    }
}

/// Teto de um script de navegador sem `timeoutMin` proprio (upload, whoami, scrape).
const DEFAULT_LIMIT_SECS: u64 = 30 * 60;

/// Um navegador anonimo (Scrapling) por vez. Trava separada da do Edge logado: o scout publico nao espera
/// login/upload, mas tambem nunca abre dois anonimos juntos no mesmo IP.
fn lock_web() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(1))
}

/// Roda `scripts/tiktok_web.py <modo>` (Scrapling, sem conta) no venv. Mesmo contrato dos .mjs.
pub async fn run_web<T: DeserializeOwned>(mode: &str, payload: &Value) -> Result<T> {
    let root = crate::config::root();
    let py = crate::recreate::venv_bin("python");
    anyhow::ensure!(py.exists(), "venv sem python: {}", py.display());
    let channel = payload["profile"].as_str().unwrap_or("default").to_string();
    let _permit = lock_web().acquire().await.map_err(|_| anyhow!("trava do navegador anonimo fechada"))?;
    let mut cmd = Command::new(py);
    cmd.arg("-I")
        .arg("-X")
        .arg("utf8")
        .arg(root.join("scripts").join("tiktok_web.py"))
        .arg(mode)
        .arg("--json")
        .arg(payload.to_string())
        .current_dir(&root);
    let script = format!("tiktok_web {mode}");
    let label = format!("Abrindo navegador anônimo ({mode})");
    tokio::time::timeout(std::time::Duration::from_secs(DEFAULT_LIMIT_SECS), exec(&channel, &script, &label, cmd))
        .await
        .map_err(|_| anyhow!("{script} passou de {} min: navegador fechado", DEFAULT_LIMIT_SECS / 60))?
}

/// Executa o processo, repassa os eventos do stderr e le o JSON da ultima linha do stdout.
async fn exec<T: DeserializeOwned>(channel: &str, script: &str, label: &str, mut cmd: Command) -> Result<T> {
    let span = crate::events::step(channel, "browser");
    span.action(label, "edge");
    tracing::info!("browser: {script}");

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let plan = crate::proc::chaos_gate(&cmd).await.with_context(|| format!("executando {script}"))?;
    let mut child = cmd.spawn().with_context(|| format!("executando {script}"))?;
    // node -> Edge/Chromium: cancelar/estourar o tempo mata o navegador junto (perfil nao fica preso)
    let _tree = crate::proc::Tree::adopt(&child);
    let _chaos = crate::chaos::Killer::arm(&plan, child.id());

    let mut stdout = child.stdout.take().ok_or_else(|| anyhow!("sem stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("sem stderr"))?;

    let out_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s).await;
        s
    });
    let mut log_tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(l) {
                if v.get("evt").is_some() {
                    if let Some(a) = v["action"].as_str() {
                        span.action(a, "edge");
                    }
                    if let Some(p) = v["progress"].as_f64() {
                        span.progress(p);
                    }
                    if let Some(u) = v["url"].as_str() {
                        span.info(&format!("url {u}"));
                    }
                    continue;
                }
            }
        }
        tracing::debug!("{l}");
        span.info(l);
        log_tail.push_back(l.to_string());
        if log_tail.len() > 40 {
            log_tail.pop_front();
        }
    }
    let status = child.wait().await?;
    let stdout = out_task.await.unwrap_or_default();

    let last = stdout.lines().rev().find(|l| l.trim_start().starts_with('{'));
    let Some(last) = last else {
        let msg = format!("{script} nao devolveu JSON ({status}). stderr:\n{}", Vec::from(log_tail).join("\n"));
        span.fail(&msg);
        anyhow::bail!(msg);
    };
    let r = serde_json::from_str(last).with_context(|| format!("parse do JSON de {script}: {last}"));
    match &r {
        Ok(_) => span.ok(""),
        Err(e) => span.fail(&e.to_string()),
    }
    r
}
