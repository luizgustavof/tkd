use crate::{agent, config::Channel, spec::Spec, state::Store};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn specs_dir() -> PathBuf {
    crate::config::root().join("out").join("specs")
}

pub fn spec_path(id: &str) -> PathBuf {
    specs_dir().join(format!("{id}.json"))
}

/// Arquivos disponiveis em video/public/<slug>/ (caminhos relativos a public/).
pub fn assets(ch: &Channel) -> Vec<String> {
    let base = crate::config::root().join("video").join("public");
    let dir = base.join(&ch.slug);
    if !dir.exists() {
        return vec![];
    }
    walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            e.path()
                .strip_prefix(&base)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .filter(|p| {
            let low = p.to_lowercase();
            [".mp4", ".webm", ".mov", ".png", ".jpg", ".jpeg", ".webp", ".mp3", ".wav", ".m4a"]
                .iter()
                .any(|e| low.ends_with(e))
        })
        .collect()
}

/// Resumo das specs ja aprovadas — exemplares que guiam o proximo video.
fn library_digest(ch: &Channel, max: usize) -> String {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(ch.library_dir()) {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        dirs.reverse();
        for d in dirs.into_iter().take(max) {
            let Ok(text) = std::fs::read_to_string(d.join("spec.json")) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let score = std::fs::read_to_string(d.join("score.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .map(|s| s["score"].to_string())
                .unwrap_or_else(|| "?".into());
            let texts: Vec<String> = v["scenes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|s| s["text"].as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default();
            items.push(format!(
                "- [{}] nota {} | {} | cenas: {}",
                v["id"].as_str().unwrap_or(""),
                score,
                v["title"].as_str().unwrap_or(""),
                texts.join(" / ")
            ));
        }
    }
    if items.is_empty() {
        "(biblioteca vazia — este e o primeiro video do canal)".into()
    } else {
        items.join("\n")
    }
}

pub async fn ideate(ch: &Channel, store: &Store, theme: Option<&str>) -> Result<Spec> {
    let span = crate::events::step(&ch.slug, "ideate");
    span.action("Criando a ideia", "claude");
    let r = ideate_inner(ch, store, theme).await;
    span.finish(r, |s| s.title.clone())
}

async fn ideate_inner(ch: &Channel, store: &Store, theme: Option<&str>) -> Result<Spec> {
    anyhow::ensure!(
        ch.style_card().exists(),
        "{} ainda não estudou nenhuma referência: adicione um canal pra estudar e clique em Mandar estudar          (na CLI: `tkd study --channel {}`)",
        ch.slug,
        ch.slug
    );
    let id = format!("{}-{}", ch.slug, Utc::now().format("%Y%m%d-%H%M%S"));
    let assets = assets(ch);
    let preset = std::fs::read_to_string(ch.preset()).unwrap_or_else(|_| "{}".into());
    let duration_hint = serde_json::from_str::<serde_json::Value>(&preset)
        .ok()
        .and_then(|p| p["duration_s"].as_f64())
        .map(|d| format!("{:.0}", d.clamp(10.0, 30.0)))
        .unwrap_or_else(|| "15".into());

    let recent = store.recent_captions(&ch.slug, 20)?;
    let recent = if recent.is_empty() {
        "(nenhuma)".to_string()
    } else {
        recent.join("\n")
    };
    let assets_list = if assets.is_empty() {
        "(nenhuma — o video deve ser puro texto/motion)".to_string()
    } else {
        assets
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("id", id.clone());
    vars.insert("style_card", std::fs::read_to_string(ch.style_card())?);
    vars.insert("preset", preset);
    vars.insert("library", library_digest(ch, 5));
    vars.insert("recent", recent);
    let fb = store.recent_review_problems(&ch.slug, 3)?;
    vars.insert(
        "feedback",
        if fb.is_empty() {
            "(nenhuma)".to_string()
        } else {
            fb.join("
")
        },
    );
    vars.insert("assets", assets_list);
    vars.insert("notes", ch.notes_text());
    vars.insert(
        "hashtags",
        if ch.hashtags.is_empty() { "(nenhuma ainda)".to_string() } else { ch.hashtags.join(" ") },
    );
    vars.insert("tracks", crate::music::digest(ch));
    vars.insert(
        "theme",
        theme.unwrap_or("livre, dentro do nicho do canal").to_string(),
    );
    vars.insert("duration_hint", duration_hint);

    let prompt = agent::render_template("ideate.md", &vars)?;
    let mut spec: Spec = agent::ask_json(ch, "ideate", &prompt, &["Read"], 420, 2).await?;
    spec.id = id.clone();
    spec.channel = ch.slug.clone();
    // duracao real de cada clipe: trava contra cena congelada
    let clip_len: std::collections::HashMap<String, f64> = crate::footage::load_bank(ch)
        .into_iter()
        .map(|c| (c.file, c.duration))
        .collect();
    spec.validate_and_fix(&assets, &clip_len)?;

    // trilha: se o agente nao escolheu, pega a primeira do banco do canal
    let track_list = crate::music::load_tracks(ch);
    if spec.audio.is_none() {
        if let Some(t) = track_list.first() {
            spec.audio = Some(t.file.clone());
            // entra no drop: e onde a musica tem mais energia
            spec.audio_start = Some((t.drop_at - 2.0).max(0.0));
        }
    }
    if let Some(t) = spec.audio.clone().and_then(|f| crate::music::beats_of(ch, &f)) {
        crate::music::snap_to_beats(&mut spec, &t, &clip_len);
    }
    // marca d'agua e do canal, nao do roteiro: vem do preset
    if spec.watermark.is_none() {
        if let Ok(p) = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(ch.preset()).unwrap_or_default(),
        ) {
            let w = &p["typography"]["watermark"];
            if let Some(t) = w["text"].as_str() {
                spec.watermark = Some(crate::spec::Watermark {
                    text: t.to_string(),
                    size_vh: w["size_vh"].as_f64().unwrap_or(2.2),
                    opacity: w["opacity"].as_f64().unwrap_or(0.55),
                    color: w["color"].as_str().unwrap_or("#bfbfbf").to_string(),
                });
            }
        }
    }

    // ultima barreira: nenhum nome/@ das referencias na legenda, nos textos ou na marca d'agua
    let mut v = serde_json::to_value(&spec)?;
    let n = crate::brand::scrub_json(ch, &mut v);
    if n > 0 {
        tracing::warn!("[{}] spec citava as referencias {n}x: trocado pela marca do canal", ch.slug);
        spec = serde_json::from_value(v)?;
    }

    std::fs::create_dir_all(specs_dir())?;
    std::fs::write(spec_path(&id), serde_json::to_string_pretty(&spec)?)?;
    store.record_video(&id, &ch.slug, &serde_json::to_string(&spec)?, "")?;
    store.log(&ch.slug, "ideate", "ok", &spec.title)?;
    crate::agent::consume_once_notes(ch);
    tracing::info!(
        "[{}] spec {id}: {} ({:.1}s, {} cenas)",
        ch.slug,
        spec.title,
        spec.duration_s,
        spec.scenes.len()
    );
    Ok(spec)
}

/// id mais recente com spec gravada para o canal.
pub fn latest_id(ch: &Channel) -> Option<String> {
    let mut ids: Vec<String> = std::fs::read_dir(specs_dir())
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .filter(|s| s.starts_with(&format!("{}-", ch.slug)))
        .collect();
    ids.sort();
    ids.pop()
}
