use crate::{config::Channel, ideate, spec::Spec, state::Store};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub fn mp4_path(id: &str) -> PathBuf {
    crate::config::root().join("out").join("renders").join(format!("{id}.mp4"))
}

pub fn load_spec(id: &str) -> Result<Spec> {
    let p = ideate::spec_path(id);
    let text = std::fs::read_to_string(&p).with_context(|| format!("lendo {}", p.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// Ultima fracao "a/b" da linha (progresso do Remotion: "Rendered 45/300").
pub fn parse_fraction(line: &str) -> Option<f64> {
    let b = line.as_bytes();
    let mut best = None;
    for (i, c) in b.iter().enumerate() {
        if *c != b'/' {
            continue;
        }
        let mut s = i;
        while s > 0 && b[s - 1].is_ascii_digit() {
            s -= 1;
        }
        let mut e = i + 1;
        while e < b.len() && b[e].is_ascii_digit() {
            e += 1;
        }
        if s < i && e > i + 1 {
            let a: f64 = line[s..i].parse().ok()?;
            let t: f64 = line[i + 1..e].parse().ok()?;
            if t > 0.0 && a <= t {
                best = Some(a / t * 100.0);
            }
        }
    }
    best
}

pub async fn render(ch: &Channel, store: &Store, id: &str) -> Result<PathBuf> {
    let span = crate::events::step(&ch.slug, "render");
    let r = render_inner(ch, store, id, &span).await;
    span.finish(r, |p| p.display().to_string())
}

async fn render_inner(ch: &Channel, store: &Store, id: &str, span: &crate::events::Span) -> Result<PathBuf> {
    let spec = load_spec(id)?;
    let out = mp4_path(id);
    std::fs::create_dir_all(out.parent().ok_or_else(|| anyhow!("caminho sem pasta"))?)?;
    let props = ideate::spec_path(id);

    tracing::info!("[{}] render {} ({:.1}s)", ch.slug, id, spec.duration_s);
    span.action(&format!("Montando o vídeo ({:.0} s)", spec.duration_s), "remotion");
    // no Windows o executavel e npx.cmd
    let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
    // renderiza num temporario e so renomeia no fim: render morto no meio nao deixa mp4 pela metade
    // com o nome final (o review/biblioteca usariam como se estivesse bom)
    let part = out.with_extension("part.mp4");
    let _ = std::fs::remove_file(&part);
    let _cleanup = crate::proc::RemoveOnDrop(part.clone());
    sweep_render_tmp();
    let render_tmp = crate::proc::RemoveOnDrop(std::env::temp_dir().join(format!("tkd-render-{id}-{}", std::process::id())));
    std::fs::create_dir_all(&render_tmp.0).with_context(|| format!("criando {}", render_tmp.0.display()))?;
    let mut cmd = Command::new(npx);
    cmd.args([
        "remotion",
        "render",
        &ch.composition,
        &part.to_string_lossy(),
        &format!("--props={}", props.to_string_lossy()),
        "--log=info",
        // qualidade alta para todo render (regra global: docs/QUALITY_RULES.md)
        "--crf=16",
        "--x264-preset=slow",
        "--jpeg-quality=95",
        "--audio-bitrate=320k",
    ])
    .current_dir(crate::config::root().join("video"))
    // TEMP so deste render (perfil do Chromium, cache do node, quadros): render morto deixava
    // `puppeteer_dev_chrome_profile-*` no %TEMP% para sempre. Fica dentro do TEMP de sempre e some ao sair.
    .env("TEMP", &render_tmp.0)
    .env("TMP", &render_tmp.0)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    crate::proc::background(&mut cmd);
    let plan = crate::proc::chaos_gate(&cmd).await?;
    let mut child = cmd.spawn().context("executando `npx remotion render`")?;
    // npx.cmd -> node -> Chromium: cancelar mata a arvore inteira
    let tree = crate::proc::Tree::adopt(&child);
    let _chaos = crate::chaos::Killer::arm(&plan, child.id());

    let stderr = child.stderr.take();
    let err_task = tokio::spawn(async move {
        let mut tail = Vec::new();
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tail.push(l);
                if tail.len() > 30 {
                    tail.remove(0);
                }
            }
        }
        tail
    });
    let stdout = child.stdout.take();
    let read_out = async move {
        let Some(o) = stdout else { return };
        // o Remotion reescreve a linha com \r; separa pelos dois
        let mut r = BufReader::new(o);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = r.read_until(b'\r', &mut buf).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            for part in String::from_utf8_lossy(&buf).split(['\r', '\n']) {
                let low = part.to_lowercase();
                if low.contains("render") || low.contains("encod") || low.contains("stitch") {
                    if let Some(p) = parse_fraction(part) {
                        let stage = if low.contains("encod") || low.contains("stitch") { 0.9 } else { 0.0 };
                        span.progress(if stage > 0.0 { 90.0 + p * 0.1 } else { p * 0.9 });
                    }
                }
            }
        }
    };
    tokio::pin!(read_out);
    // stdout fechou primeiro (erro rapido: disco cheio, etc.): nao esperar `read_out` de novo. Antes isso dava
    // panic "`async fn` resumed after completion" e o tkd caia em vez de dar a mensagem do render.
    let (status, out_done) = tokio::select! {
        st = child.wait() => (st?, false),
        _ = &mut read_out => (child.wait().await?, true),
    };
    // npx.cmd saiu (ou morreu) e o node/Chromium seguem segurando o pipe: derruba a arvore em vez de
    // esperar o render orfao terminar sozinho
    if !out_done && tokio::time::timeout(std::time::Duration::from_secs(2), &mut read_out).await.is_err() {
        tracing::warn!("[{}] npx saiu ({status}) com o render ainda rodando: derrubando node/Chromium", ch.slug);
        tree.terminate();
    }
    let tail = tokio::time::timeout(std::time::Duration::from_secs(5), err_task)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let why = if tail.iter().all(|l| l.trim().is_empty()) {
        "o processo morreu sem mensagem (fechado de fora ou sem memoria)".to_string()
    } else {
        tail.join("\n")
    };
    anyhow::ensure!(status.success(), "remotion render falhou ({status}): {why}");
    anyhow::ensure!(part.exists(), "remotion nao gerou {}", out.display());
    std::fs::rename(&part, &out).with_context(|| format!("trocando {}", out.display()))?;

    store.record_video(id, &ch.slug, &serde_json::to_string(&spec)?, &out.to_string_lossy())?;
    store.log(&ch.slug, "render", "ok", id)?;
    tracing::info!("[{}] mp4: {}", ch.slug, out.display());
    Ok(out)
}

/// Pastas `tkd-render-<id>-<pid>` de renders cujo processo ja morreu (app fechado no meio nao roda o Drop).
fn sweep_render_tmp() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(rest) = n.strip_prefix("tkd-render-") else { continue };
        let Some(pid) = rest.rsplit('-').next().and_then(|p| p.parse::<u32>().ok()) else { continue };
        if pid != std::process::id() && !crate::proc::pid_alive(pid) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fraction() {
        assert_eq!(super::parse_fraction("Rendered 45/90, time remaining 3s"), Some(50.0));
        assert_eq!(super::parse_fraction("sem nada"), None);
    }
}
