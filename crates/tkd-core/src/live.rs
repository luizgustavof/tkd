//! `tkd live`: assiste uma live (Twitch, Kick, YouTube), le o chat sem login e corta os momentos fortes.
//! O trabalho pesado fica em scripts/live.py (streamlink -> segmentos de 30 s -> moments.py perfil "live").

use crate::proc::TreeExt;
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

pub struct LiveOpts {
    pub url: String,
    pub minutes: f64,
    pub threshold: f64,
    pub min: f64,
    pub max: f64,
    pub lang: String,
    pub no_chat: bool,
    pub captions: bool,
    /// so corte bruto durante a live (whisper/9:16 depois, so no que vai ao ar)
    pub lazy: bool,
    /// jogos aceitos (categoria da live); vazio = qualquer
    pub games: Vec<String>,
    /// canal dono (Ao vivo agrupa a live embaixo dele); vazio = "live"
    pub channel: String,
}

/// streamlink do venv; `TKD_STREAMLINK` de fora vale (os experimentos de caos usam um streamlink falso que nao
/// sai na rede, tanto na captura quanto no "esta ao vivo?" do vigia).
pub fn streamlink_bin() -> PathBuf {
    std::env::var_os("TKD_STREAMLINK").map(PathBuf::from).unwrap_or_else(|| crate::recreate::venv_bin("streamlink"))
}

/// Pasta de saida: out/live/<plataforma>-<canal>-<data>.
fn session_dir(url: &str) -> PathBuf {
    let u = url.to_lowercase();
    let plat = if u.contains("twitch.tv") {
        "twitch"
    } else if u.contains("kick.com") {
        "kick"
    } else if u.contains("youtu") {
        "youtube"
    } else {
        "live"
    };
    // youtube.com/@canal/live -> canal; demais: ultimo pedaco da URL (twitch.tv/canal, watch?v=id)
    let base = match u.split_once('@') {
        Some((_, r)) => r.split(['/', '?']).next().unwrap_or("canal"),
        None => u.trim_end_matches('/').rsplit(['/', '=']).next().unwrap_or("canal"),
    };
    let test_channel: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(30)
        .collect();
    crate::config::root()
        .join("out")
        .join("live")
        .join(format!("{plat}-{test_channel}-{}", chrono::Local::now().format("%Y%m%d-%H%M")))
}

pub async fn run_live(o: LiveOpts) -> Result<()> {
    // uma etapa por streamer ("live:streamer_a"): com a mesma etapa as lives simultaneas se sobrescreviam no Ao vivo
    let agent = if o.channel.is_empty() { "live" } else { o.channel.as_str() };
    let span = crate::events::step(agent, &format!("live:{}", crate::clips::streamer_name(&o.url)));
    span.action(&format!("Assistindo live {}", o.url), "python");
    let r = run_live_inner(o).await;
    span.finish(r, |n| format!("{n} clipes"))?;
    Ok(())
}

async fn run_live_inner(o: LiveOpts) -> Result<usize> {
    crate::media::check_ffmpeg().await;
    let out = session_dir(&o.url);
    std::fs::create_dir_all(&out)?;
    sweep_stale_bufs();
    prune_old_sessions(keep_days());
    // o live.py so apaga o buf no fim natural da live; vigia parado, piloto desligado ou app fechado matam a
    // arvore e o buf ficava (~200 MB a cada 15 min de live)
    let _buf = (std::env::var("TKD_LIVE_KEEP_BUF").as_deref() != Ok("1")).then(|| crate::proc::RemoveOnDrop(out.join("buf")));
    let mut args: Vec<String> = vec![
        "-I".into(), "-X".into(), "utf8".into(), "scripts/live.py".into(), o.url.clone(), out.to_string_lossy().to_string(),
        "--minutes".into(), o.minutes.to_string(), "--threshold".into(), o.threshold.to_string(),
        "--min".into(), o.min.to_string(), "--max".into(), o.max.to_string(), "--lang".into(), o.lang.clone(),
    ];
    if o.captions {
        args.push("--captions".into());
    }
    if o.no_chat {
        args.push("--no-chat".into());
    }
    if o.lazy {
        args.push("--lazy".into());
    }
    if !o.games.is_empty() {
        args.push("--games".into());
        args.push(o.games.join("|"));
    }
    println!("live: {} -> {}", o.url, out.display());
    let st = Command::new(crate::recreate::venv_bin("python"))
        .args(&args)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("TKD_FFMPEG", crate::media::ffmpeg_bin())
        .env("TKD_STREAMLINK", streamlink_bin())
        .current_dir(crate::config::root())
        .tree_status()
        .await
        .context("scripts/live.py")?;
    anyhow::ensure!(st.success(), "live.py terminou com erro ({st})");
    let n = std::fs::read_dir(out.join("clips"))
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
        .unwrap_or(0);
    println!("{n} clipes em {}", out.join("clips").display());
    Ok(n)
}

/// Apaga `buf/` de sessoes de live mortas (processo derrubado, reboot): uma captura viva grava segmento a cada
/// poucos segundos, entao buf parado ha mais de 1 h e lixo.
fn sweep_stale_bufs() {
    let Ok(sessions) = std::fs::read_dir(crate::config::root().join("out").join("live")) else { return };
    for s in sessions.flatten() {
        let buf = s.path().join("buf");
        let newest = std::fs::read_dir(&buf)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
            .chain(std::fs::metadata(&buf).and_then(|m| m.modified()).ok())
            .max();
        if newest.and_then(|t| t.elapsed().ok()).is_some_and(|d| d > std::time::Duration::from_secs(3600)) {
            if std::fs::remove_dir_all(&buf).is_ok() {
                tracing::info!("buf de live antiga apagado: {}", buf.display());
            }
        }
    }
}

/// Dias que uma sessao de live fica em disco depois do ultimo arquivo gravado (`TKD_LIVE_KEEP_DAYS`, padrao 3;
/// 0 = nunca apaga). Corte com mais de `cortes::MAX_AGE_H` ja nao vai ao ar, entao 3 dias da folga.
pub fn keep_days() -> u64 {
    std::env::var("TKD_LIVE_KEEP_DAYS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(3)
}

/// Apaga sessoes inteiras de `out/live` (clipes, chat) cujo arquivo mais novo tem mais de `days` dias.
/// Antes quem limpava era o backup da nuvem (cloud/sync.mjs), que saiu. Devolve quantas sessoes sairam.
pub fn prune_old_sessions(days: u64) -> usize {
    if days == 0 {
        return 0;
    }
    let limit = std::time::Duration::from_secs(days * 86_400);
    let Ok(sessions) = std::fs::read_dir(crate::config::root().join("out").join("live")) else { return 0 };
    let mut n = 0;
    for s in sessions.flatten().filter(|s| s.path().is_dir()) {
        let newest = walkdir::WalkDir::new(s.path())
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
            .max()
            // sessao sem nenhum arquivo: vale a data da pasta
            .or_else(|| s.metadata().ok().and_then(|m| m.modified().ok()));
        if newest.and_then(|t| t.elapsed().ok()).is_some_and(|d| d > limit) && std::fs::remove_dir_all(s.path()).is_ok() {
            tracing::info!("sessao de live antiga apagada ({days}+ dias): {}", s.path().display());
            n += 1;
        }
    }
    n
}
