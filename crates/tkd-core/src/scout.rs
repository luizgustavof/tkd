use crate::proc::TreeExt;
use crate::{browser, config::Channel, media, state::{RefVideo, Store}};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
struct ScrapeOut {
    ok: bool,
    #[serde(default)]
    error: String,
    #[serde(default)]
    account: serde_json::Value,
    #[serde(default)]
    photos: usize,
    #[serde(default)]
    videos: Vec<ScrapedVideo>,
    /// muro anti-bot (scripts/tiktok_web.py); o .mjs nao manda
    #[serde(default)]
    walled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScrapedVideo {
    id: String,
    url: String,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    views: String,
}

#[derive(Debug, Default, Deserialize)]
struct DownloadOut {
    #[serde(default)]
    downloaded: Vec<Downloaded>,
    #[serde(default)]
    failed: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Downloaded {
    id: String,
    url: String,
    file: String,
    #[serde(default)]
    caption: String,
    #[serde(default)]
    music: String,
    #[serde(default)]
    likes: String,
    #[serde(default)]
    comments: String,
    #[serde(default)]
    shares: String,
    #[serde(default)]
    hashtags: Vec<String>,
}

/// Varre o canal de referencia, baixa o que e novo e grava a analise tecnica.
pub async fn scout(ch: &Channel, store: &Store, limit: usize) -> Result<usize> {
    let span = crate::events::step(&ch.slug, "scout");
    span.action("Buscando referências novas", "edge");
    let r = scout_inner(ch, store, limit, None).await;
    span.finish(r, |n| format!("{n} referências novas"))
}

/// Mesmo que `scout`, mas so num perfil da lista (botao "Estudar" de cada linha).
pub async fn scout_one(ch: &Channel, store: &Store, limit: usize, handle: &str) -> Result<usize> {
    let span = crate::events::step(&ch.slug, "scout");
    span.action(&format!("Buscando vídeos novos de @{handle}"), "edge");
    let r = scout_inner(ch, store, limit, Some(handle)).await;
    span.finish(r, |n| format!("{n} referências novas"))
}

async fn scout_inner(ch: &Channel, store: &Store, limit: usize, only: Option<&str>) -> Result<usize> {
    let all = ch.ref_handles();
    let handles: Vec<String> = match only {
        Some(h) => {
            let h = h.trim().trim_start_matches('@');
            anyhow::ensure!(all.iter().any(|x| x.eq_ignore_ascii_case(h)), "[{}] @{h} nao esta na lista do canal", ch.slug);
            vec![h.to_string()]
        }
        None => all.clone(),
    };
    anyhow::ensure!(!handles.is_empty(), "[{}] sem canal para estudar — adicione uma inspiracao", ch.slug);
    // a principal leva a maior parte; as inspiracoes dividem o resto
    let mut total = 0;
    let mut error_list = Vec::new();
    for (i, h) in handles.iter().enumerate() {
        // a principal (1a da lista do canal) leva mais videos que as inspiracoes
        let main = all.first().is_some_and(|m| m.eq_ignore_ascii_case(h));
        let lim = if main || i == 0 && only.is_some() { limit } else { (limit / 2).max(2) };
        match scout_handle(ch, store, h, lim, main).await {
            Ok(n) => total += n,
            Err(e) => {
                tracing::error!("[{}] scout @{h}: {e}", ch.slug);
                error_list.push(format!("@{h}: {e}"));
            }
        }
    }
    if total == 0 && error_list.len() == handles.len() {
        anyhow::bail!("{}", error_list.join("; "));
    }
    Ok(total)
}

async fn scout_handle(ch: &Channel, store: &Store, handle: &str, limit: usize, main: bool) -> Result<usize> {
    tracing::info!("[{}] scout @{handle}", ch.slug);

    let res = list_grid(ch, handle, limit * 3).await?;

    if !res.ok {
        store.log(&ch.slug, "scout", "erro", &res.error)?;
        anyhow::bail!("[{}] scrape falhou: {}", ch.slug, res.error);
    }

    std::fs::create_dir_all(ch.knowledge_dir())?;
    std::fs::write(
        ch.knowledge_dir().join(if main { "profile.json".to_string() } else { format!("profile-{handle}.json") }),
        serde_json::to_string_pretty(&res.account)?,
    )?;

    let mut new_items = Vec::new();
    for v in &res.videos {
        if store.ref_downloaded(&v.id)? {
            continue;
        }
        store.insert_ref(&RefVideo {
            id: v.id.clone(),
            channel: ch.slug.clone(),
            url: v.url.clone(),
            caption: v.caption.clone(),
            stats: json!({ "views": v.views }).to_string(),
            downloaded: false,
            analyzed: false,
        })?;
        new_items.push(v.clone());
    }
    new_items.truncate(limit);
    tracing::info!(
        "[{}] {} videos na grade ({} posts de foto ignorados), {} novos",
        ch.slug, res.videos.len(), res.photos, new_items.len()
    );
    if new_items.is_empty() {
        store.log(&ch.slug, "scout", "ok", "nada novo")?;
        return Ok(0);
    }

    std::fs::create_dir_all(ch.refs_dir())?;
    std::fs::create_dir_all(ch.ref_meta_dir())?;

    let dl = download_cascade(ch, handle, &new_items).await;
    for f in &dl.failed {
        tracing::warn!("[{}] download falhou: {}", ch.slug, f);
    }

    let mut n = 0;
    for d in &dl.downloaded {
        store.mark_ref(&d.id, true, false)?;
        let path = std::path::PathBuf::from(&d.file);
        let analysis = match media::analyze(&path, &ch.knowledge_dir().join("shots")).await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("[{}] analise de {} falhou: {e}", ch.slug, d.id);
                continue;
            }
        };
        if analysis.is_slideshow {
            tracing::warn!("[{}] {} e carrossel de fotos — descartado", ch.slug, d.id);
            let _ = std::fs::remove_file(&path);
            store.mark_ref(&d.id, true, true)?;
            continue;
        }
        let meta = json!({
            "id": d.id,
            "url": d.url,
            "file": d.file,
            "caption": d.caption,
            "music": d.music,
            "stats": { "likes": d.likes, "comments": d.comments, "shares": d.shares },
            "hashtags": d.hashtags,
            "analysis": analysis,
        });
        std::fs::write(
            ch.ref_meta_dir().join(format!("{}.json", d.id)),
            serde_json::to_string_pretty(&meta)?,
        )?;
        store.mark_ref(&d.id, true, true)?;
        n += 1;
        tracing::info!(
            "[{}] {} analisado: {:.1}s, {} planos, mov {:.2}, {} batidas",
            ch.slug, d.id, analysis.probe.duration, analysis.shots.len(),
            analysis.motion_avg, analysis.audio.hits.len()
        );
    }

    store.log(&ch.slug, "scout", "ok", &format!("{n} refs novas"))?;
    Ok(n)
}

/// Grade do perfil: Scrapling anonimo (scripts/tiktok_web.py) primeiro; muro ou erro cai no Edge logado (.mjs).
async fn list_grid(ch: &Channel, handle: &str, limit: usize) -> Result<ScrapeOut> {
    let payload = json!({ "handle": handle, "limit": limit, "profile": ch.slug });
    match browser::run_web::<ScrapeOut>("channel", &payload).await {
        Ok(r) if r.ok => return Ok(r),
        // muro e por IP: abrir o Edge logado logo em seguida so estende o bloqueio
        Ok(r) if r.walled => return Ok(r),
        Ok(r) => tracing::warn!("[{}] scrapling @{handle}: {} — tentando o Edge logado", ch.slug, r.error),
        Err(e) => tracing::warn!("[{}] scrapling @{handle}: {e} — tentando o Edge logado", ch.slug),
    }
    browser::run("scrape_channel.mjs", &payload).await
}

/// Download em cascata: yt-dlp direto no link -> Scrapling anonimo -> Edge logado (.mjs).
/// Cada etapa so recebe o que a anterior nao conseguiu.
async fn download_cascade(ch: &Channel, handle: &str, new_items: &[ScrapedVideo]) -> DownloadOut {
    let dir = ch.refs_dir();
    let mut out = DownloadOut::default();
    let mut missing: Vec<&ScrapedVideo> = Vec::new();
    for (i, v) in new_items.iter().enumerate() {
        if i > 0 {
            // pausa humana entre videos; o TikTok tambem conta requisicoes por IP
            let ms = 1500 + rand::random::<u64>() % 2000;
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        }
        match ytdlp_tiktok(v, &dir).await {
            Ok(d) => out.downloaded.push(d),
            Err(e) => {
                tracing::info!("[{}] yt-dlp {}: {e}", ch.slug, v.id);
                missing.push(v);
            }
        }
    }
    for (script, web) in [("download", true), ("download.mjs", false)] {
        if missing.is_empty() {
            break;
        }
        let payload = json!({
            "videos": missing.iter().map(|v| json!({"id": v.id, "url": v.url})).collect::<Vec<_>>(),
            "outDir": dir.to_string_lossy(),
            "profile": ch.slug,
            "handle": handle,
        });
        let r: Result<DownloadOut> =
            if web { browser::run_web(script, &payload).await } else { browser::run(script, &payload).await };
        match r {
            Ok(r) => {
                missing.retain(|v| !r.downloaded.iter().any(|d| d.id == v.id));
                out.downloaded.extend(r.downloaded);
                if missing.is_empty() || !web {
                    out.failed.extend(r.failed);
                }
            }
            Err(e) => {
                tracing::warn!("[{}] {script}: {e}", ch.slug);
                if !web {
                    out.failed.extend(missing.iter().map(|v| json!({"id": v.id, "error": e.to_string()})));
                }
            }
        }
    }
    out
}

/// yt-dlp no link publico do video. Metadados do proprio yt-dlp (legenda, musica, contagens, hashtags da legenda).
async fn ytdlp_tiktok(v: &ScrapedVideo, dir: &std::path::Path) -> Result<Downloaded> {
    let tpl = dir.join(format!("{}.%(ext)s", v.id));
    let mut cmd = tokio::process::Command::new("yt-dlp");
    cmd.args(["--no-playlist", "--no-warnings", "-q", "--no-simulate", "-f", "b[ext=mp4]/b", "--remux-video", "mp4"])
        .args(["--print", "%(.{description,track,artist,like_count,comment_count,repost_count})j"])
        .arg("-o")
        .arg(&tpl)
        .arg(&v.url);
    if let Ok(ff) = std::env::var("TKD_FFMPEG") {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    crate::proc::background(&mut cmd);
    // teto: download travado segurava o scout do canal para sempre
    let o = cmd.tree_output_timeout(std::time::Duration::from_secs(15 * 60)).await.context("yt-dlp do TikTok")?;
    let file = dir.join(format!("{}.mp4", v.id));
    if !o.status.success() || !file.exists() {
        let err = String::from_utf8_lossy(&o.stderr);
        anyhow::bail!("{}", err.lines().filter(|l| l.contains("ERROR")).last().unwrap_or(err.trim()));
    }
    if std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0) < 50_000 {
        let _ = std::fs::remove_file(&file);
        anyhow::bail!("arquivo vazio");
    }
    let m: serde_json::Value = String::from_utf8_lossy(&o.stdout)
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str(l).ok())
        .unwrap_or_default();
    let caption = m["description"].as_str().unwrap_or(&v.caption).to_string();
    let n = |k: &str| m[k].as_u64().map(fmt_n).unwrap_or_default();
    // o track do TikTok ja vem "titulo - autor"
    let music = m["track"].as_str().or(m["artist"].as_str()).unwrap_or_default().to_string();
    let hashtags = caption
        .split_whitespace()
        .filter(|w| w.starts_with('#') && w.len() > 1)
        .map(|w| w.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_').to_string())
        .take(15)
        .collect();
    Ok(Downloaded {
        id: v.id.clone(),
        url: v.url.clone(),
        file: file.to_string_lossy().to_string(),
        caption,
        music,
        likes: n("like_count"),
        comments: n("comment_count"),
        shares: n("repost_count"),
        hashtags,
    })
}

/// 1234567 -> "1.2M", como a pagina mostra (o resto do tkd le as contagens nesse formato).
fn fmt_n(n: u64) -> String {
    for (lim, suf) in [(1e9, "B"), (1e6, "M"), (1e3, "K")] {
        if n as f64 >= lim {
            let s = format!("{:.1}", n as f64 / lim);
            return format!("{}{suf}", s.trim_end_matches(".0"));
        }
    }
    n.to_string()
}
