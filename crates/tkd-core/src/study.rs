use crate::proc::TreeExt;
use crate::{agent, config::Channel, media, state::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Serialize)]
pub struct StudyOut {
    pub persona: String,
    pub niche: String,
    pub style_card_md: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
    pub preset: serde_json::Value,
}

struct RefDigest {
    body_text: String,
    sheet: Option<String>,
    keyframes: Vec<String>,
}

/// Transforma a analise completa de um video em texto legivel para o agente.
fn digest(v: &serde_json::Value) -> RefDigest {
    let a = &v["analysis"];
    let p = &a["probe"];
    let audio = &a["audio"];
    let mut t = String::new();

    t.push_str(&format!(
        "### ref {}\n- {:.1}s | {}x{} @{:.0}fps | audio: {}\n- trocas de plano: {} ({:.2}/s, plano medio {:.2}s) | secas: {} | dissolves: {}\n- movimento: medio {:.3}, pico {:.3}\n- loudness: {} LUFS | batidas detectadas: {} | cortes na batida: {}\n- paleta geral: {}\n- musica: \"{}\"\n- caption: \"{}\"\n- hashtags: {} | stats: {}\n",
        v["id"].as_str().unwrap_or(""),
        p["duration"].as_f64().unwrap_or(0.0),
        p["width"], p["height"],
        p["fps"].as_f64().unwrap_or(0.0),
        p["has_audio"],
        a["transitions"].as_array().map(|c| c.len()).unwrap_or(0),
        a["cuts_per_sec"].as_f64().unwrap_or(0.0),
        a["avg_cut"].as_f64().unwrap_or(0.0),
        a["transitions"].as_array().map(|t| t.iter().filter(|x| x["kind"] == "cut").count()).unwrap_or(0),
        a["transitions"].as_array().map(|t| t.iter().filter(|x| x["kind"] == "dissolve").count()).unwrap_or(0),
        a["motion_avg"].as_f64().unwrap_or(0.0),
        a["motion_max"].as_f64().unwrap_or(0.0),
        a["lufs"],
        audio["hits"].as_array().map(|h| h.len()).unwrap_or(0),
        audio["cuts_on_beat"],
        a["palette"],
        v["music"].as_str().unwrap_or(""),
        v["caption"].as_str().unwrap_or("").replace('\n', " "),
        v["hashtags"],
        v["stats"],
    ));

    if let Some(l) = sound_line(&v["sound"]) {
        t.push_str(&l);
    }

    // tabela plano a plano: cobre o video inteiro, nao so uma amostra
    if let Some(shots) = a["shots"].as_array() {
        t.push_str("- planos (inicio s | duracao s | movimento | cores):\n");
        for s in shots {
            t.push_str(&format!(
                "    {:>5.2} | {:>5.2} | {:.3} | {}\n",
                s["start"].as_f64().unwrap_or(0.0),
                s["duration"].as_f64().unwrap_or(0.0),
                s["motion"].as_f64().unwrap_or(0.0),
                s["palette"],
            ));
        }
    }

    RefDigest {
        body_text: t,
        sheet: a["contact_sheet"].as_str().map(|s| s.to_string()),
        keyframes: a["keyframes"]
            .as_array()
            .map(|k| k.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    }
}

/// "- som: fala 8s (32%): "..." | musica: X - Y (alta) | 129 BPM | drop 6.5s"
fn sound_line(snd: &serde_json::Value) -> Option<String> {
    if !snd.is_object() {
        return None;
    }
    let sp = &snd["speech"];
    let speech_lines: Vec<String> = sp["lines"]
        .as_array()
        .map(|l| l.iter().filter_map(|x| x["text"].as_str().map(|t| format!("\"{t}\""))).take(3).collect())
        .unwrap_or_default();
    let speech_line = if speech_lines.is_empty() {
        "sem fala".to_string()
    } else {
        format!(
            "fala {:.0}s ({:.0}% do video): {}",
            sp["seconds"].as_f64().unwrap_or(0.0),
            sp["ratio"].as_f64().unwrap_or(0.0) * 100.0,
            speech_lines.join(" / ")
        )
    };
    let musica = snd["tracks"]
        .as_array()
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.iter()
                .map(|x| format!("{} - {} ({})", x["title"].as_str().unwrap_or(""), x["artist"].as_str().unwrap_or(""), x["confidence"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .unwrap_or_else(|| "nao identificada (remix/acelerada ou faixa rara)".into());
    let m = &snd["music"];
    let drop = m["drop_s"].as_f64().map(|d| format!("drop {d:.1}s")).unwrap_or_else(|| "sem drop claro".into());
    Some(format!("- som: {speech_line} | musica: {musica} | {:.0} BPM | {drop}\n", m["bpm"].as_f64().unwrap_or(0.0)))
}

/// "90.9K" -> 90900
fn count(s: &serde_json::Value) -> f64 {
    let t = s.as_str().map(|x| x.trim().to_uppercase()).unwrap_or_else(|| s.to_string());
    let (n, mult) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1e3),
        Some('M') => (&t[..t.len() - 1], 1e6),
        Some('B') => (&t[..t.len() - 1], 1e9),
        _ => (t.as_str(), 1.0),
    };
    n.replace(',', ".").parse::<f64>().map(|x| x * mult).unwrap_or(0.0)
}

/// Mede o audio das referencias que ainda nao tem (fala, BPM, drop e a musica pelo Shazam).
/// Roda no estudo, entao cobre as refs novas e as antigas. Falha numa ref nao para o estudo.
async fn ensure_ref_audio(ch: &Channel, span: &crate::events::Span) -> usize {
    let Ok(rd) = std::fs::read_dir(ch.ref_meta_dir()) else { return 0 };
    let mut metas: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    metas.sort();
    let pend: Vec<(PathBuf, serde_json::Value, PathBuf)> = metas
        .into_iter()
        .filter_map(|p| {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            if v["sound"].is_object() || v["analysis"]["is_slideshow"].as_bool().unwrap_or(false) {
                return None;
            }
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let file = v["file"]
                .as_str()
                .map(PathBuf::from)
                .filter(|f| f.exists())
                .unwrap_or_else(|| ch.refs_dir().join(format!("{id}.mp4")));
            file.exists().then_some((p, v, file))
        })
        .collect();
    let total = pend.len();
    let mut n = 0;
    for (i, (meta_p, mut v, file)) in pend.into_iter().enumerate() {
        span.action(&format!("Ouvindo a referência {} de {total}", i + 1), "python");
        let out = tokio::process::Command::new(crate::recreate::venv_bin("python"))
            .args(["-I", "-X", "utf8"])
            .arg(crate::config::root().join("scripts").join("ref_audio.py"))
            .arg(&file)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .current_dir(crate::config::root())
            .tree_output()
            .await;
        let snd = match out {
            Ok(o) if o.status.success() => serde_json::from_slice::<serde_json::Value>(&o.stdout).ok(),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).to_string();
                tracing::warn!("[{}] audio de {}: {}", ch.slug, file.display(), err.lines().last().unwrap_or(""));
                None
            }
            Err(e) => {
                tracing::warn!("[{}] audio de {}: {e}", ch.slug, file.display());
                None
            }
        };
        if let Some(s) = snd {
            v["sound"] = s;
            if std::fs::write(&meta_p, serde_json::to_string_pretty(&v).unwrap_or_default()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Repertorio do nicho: as musicas identificadas nas referencias, com quantas refs usam e
/// os likes somados. Fica em knowledge/sound-refs.json (base pra escolher trilha) e vai pro estudo.
fn repertoire(ch: &Channel) -> String {
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct T {
        title: String,
        artist: String,
        refs: Vec<String>,
        likes: f64,
        bpm: Vec<f64>,
    }
    let mut by: BTreeMap<String, T> = BTreeMap::new();
    let (mut total, mut with_speech, mut no_music) = (0, 0, 0);
    if let Ok(rd) = std::fs::read_dir(ch.ref_meta_dir()) {
        for p in rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&p).unwrap_or_default()) else { continue };
            let snd = &v["sound"];
            if !snd.is_object() {
                continue;
            }
            total += 1;
            if snd["speech"]["seconds"].as_f64().unwrap_or(0.0) >= 1.0 {
                with_speech += 1;
            }
            let tracks = snd["tracks"].as_array().cloned().unwrap_or_default();
            if tracks.is_empty() {
                no_music += 1;
            }
            for t in tracks {
                let key = t["key"].as_str().unwrap_or_default().to_string();
                let e = by.entry(key).or_default();
                e.title = t["title"].as_str().unwrap_or_default().into();
                e.artist = t["artist"].as_str().unwrap_or_default().into();
                e.refs.push(v["id"].as_str().unwrap_or_default().into());
                e.likes += count(&v["stats"]["likes"]);
                if let Some(b) = snd["music"]["bpm"].as_f64() {
                    e.bpm.push(b);
                }
            }
        }
    }
    let mut list: Vec<&T> = by.values().collect();
    list.sort_by(|a, b| b.refs.len().cmp(&a.refs.len()).then(b.likes.partial_cmp(&a.likes).unwrap_or(std::cmp::Ordering::Equal)));
    let json: Vec<serde_json::Value> = list
        .iter()
        .map(|t| serde_json::json!({ "title": t.title, "artist": t.artist, "refs": t.refs, "likes": t.likes, "bpm": t.bpm }))
        .collect();
    let _ = std::fs::write(ch.knowledge_dir().join("sound-refs.json"), serde_json::to_string_pretty(&json).unwrap_or_default());
    if total == 0 {
        return "(audio das referencias ainda nao medido)".into();
    }
    let mut s = format!(
        "{total} refs medidas: {with_speech} com fala de personagem, {} com musica identificada, {no_music} sem musica identificada.\n",
        total - no_music
    );
    for t in list.iter().take(15) {
        s.push_str(&format!("- {} - {} | {} ref(s) | {:.0} likes somados\n", t.title, t.artist, t.refs.len(), t.likes));
    }
    s
}

fn refs_digest(ch: &Channel) -> Result<(String, Vec<String>)> {
    let dir = ch.ref_meta_dir();
    let mut body_text = Vec::new();
    let mut images = Vec::new();
    if !dir.exists() {
        return Ok((String::new(), images));
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    entries.sort();

    for (i, path) in entries.iter().enumerate() {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        if v["analysis"]["is_slideshow"].as_bool().unwrap_or(false) {
            continue;
        }
        let d = digest(&v);
        body_text.push(d.body_text);
        let _ = i;
        // tudo: mosaico do video inteiro + um frame por plano de cada referencia
        if let Some(s) = d.sheet {
            images.push(s);
        }
        images.extend(d.keyframes);
    }
    Ok((body_text.join("\n"), images))
}

/// Recalcula a analise dos mp4 ja baixados (sem rebaixar nada).
pub async fn reanalyze(ch: &Channel, store: &Store) -> Result<usize> {
    let dir = ch.refs_dir();
    if !dir.exists() {
        return Ok(0);
    }
    let shots_dir = ch.knowledge_dir().join("shots");
    std::fs::create_dir_all(ch.ref_meta_dir())?;
    let mut n = 0;
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().map(|e| e != "mp4").unwrap_or(true) {
            continue;
        }
        let id = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let meta_path = ch.ref_meta_dir().join(format!("{id}.json"));
        let mut meta: serde_json::Value = std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({ "id": id, "file": path.to_string_lossy() }));

        let analysis = media::analyze(&path, &shots_dir).await?;
        if analysis.is_slideshow {
            tracing::warn!("[{}] {id} e carrossel de fotos — descartado", ch.slug);
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&meta_path);
            continue;
        }
        let plain = analysis.transitions.iter().filter(|t| t.kind == "cut").count();
        tracing::info!(
            "[{}] {id}: {:.1}s, {} planos ({} cortes secos, {} dissolves), mov {:.2}, {} batidas",
            ch.slug,
            analysis.probe.duration,
            analysis.shots.len(),
            plain,
            analysis.transitions.len() - plain,
            analysis.motion_avg,
            analysis.audio.hits.len()
        );
        meta["analysis"] = serde_json::to_value(&analysis)?;
        std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
        n += 1;
    }
    store.log(&ch.slug, "analyze", "ok", &format!("{n} refs reanalisadas"))?;
    Ok(n)
}

pub async fn study(ch: &Channel, store: &Store, force: bool) -> Result<()> {
    let span = crate::events::step(&ch.slug, "study");
    span.action("Estudando as referências", "claude");
    let r = study_inner(ch, store, force, &span).await;
    span.finish(r, |_| String::new())
}

async fn study_inner(ch: &Channel, store: &Store, force: bool, span: &crate::events::Span) -> Result<()> {
    if force || !ch.style_card().exists() {
        let n = ensure_ref_audio(ch, span).await;
        if n > 0 {
            tracing::info!("[{}] audio medido em {n} referencias", ch.slug);
        }
        span.action("Estudando as referências", "claude");
    }
    let (refs, images) = refs_digest(ch)?;
    anyhow::ensure!(
        !refs.is_empty(),
        "[{}] sem referencias analisadas — rode `tkd scout` antes",
        ch.slug
    );
    if ch.style_card().exists() && !force {
        tracing::info!("[{}] style-card ja existe (use --force para refazer)", ch.slug);
        return Ok(());
    }

    let image_list = images
        .iter()
        .map(|p| format!("- {p}"))
        .collect::<Vec<_>>()
        .join("\n");

    let profile = std::fs::read_to_string(ch.knowledge_dir().join("profile.json"))
        .unwrap_or_else(|_| "{}".into());

    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("handle", ch.ref_handles().join(", @"));
    vars.insert("profile", profile);
    vars.insert("refs", refs);
    vars.insert("frames", image_list);
    vars.insert("brand", crate::brand::brand_of(ch));
    vars.insert("repertoire", repertoire(ch));
    let prompt = agent::render_template("study.md", &vars)?;

    tracing::info!("[{}] study com {} imagens", ch.slug, images.len());
    let mut out: StudyOut = agent::ask_json(ch, "study", &prompt, &["Read", "Glob"], 900, 2).await?;

    // nada das referencias (nome, @, assinatura) vira marca do canal, mesmo que o agente escorregue
    let names = crate::brand::reference_names(ch);
    let brand = crate::brand::brand_of(ch);
    let mut swaps = 0;
    // o style card pode citar as refs pra descrever variacoes ("variante <perfil>"); quem garante
    // que nada delas sai no video e o scrub da spec/plano. Persona e nicho viram texto do canal.
    for field in [&mut out.persona, &mut out.niche] {
        let (t, n) = crate::brand::scrub_with(field, &names, &brand);
        *field = t;
        swaps += n;
    }
    {
        let mut p = out.preset.clone();
        swaps += crate::brand::scrub_json(ch, &mut p);
        out.preset = p;
    }
    let prior = out.hashtags.len();
    out.hashtags.retain(|t| !crate::brand::tag_mentions_reference(t, &names));
    swaps += prior - out.hashtags.len();
    if swaps > 0 {
        tracing::warn!("[{}] estudo citava as referencias {swaps}x: trocado por {brand}", ch.slug);
    }

    std::fs::create_dir_all(ch.knowledge_dir())?;
    std::fs::write(ch.style_card(), &out.style_card_md)?;
    std::fs::write(ch.preset(), serde_json::to_string_pretty(&out.preset)?)?;
    update_channel_toml(ch, &out).context("atualizando channel.toml")?;
    store.log(&ch.slug, "study", "ok", &out.niche)?;
    tracing::info!("[{}] style-card escrito: {}", ch.slug, out.niche);
    Ok(())
}

/// Grava persona/nicho/hashtags decididos pelo estudo de volta no channel.toml.
fn update_channel_toml(ch: &Channel, out: &StudyOut) -> Result<()> {
    crate::config::edit_toml(&ch.dir.join("channel.toml"), |doc| {
        doc["persona"] = toml_edit::value(out.persona.clone());
        doc["niche"] = toml_edit::value(out.niche.clone());
        let mut arr = toml_edit::Array::new();
        for h in &out.hashtags {
            arr.push(h.clone());
        }
        doc["hashtags"] = toml_edit::value(arr);
        Ok(())
    })
}

#[cfg(test)]
mod tests_count {
    #[test]
    fn counts_likes() {
        assert_eq!(super::count(&serde_json::json!("90.9K")), 90900.0);
        assert_eq!(super::count(&serde_json::json!("1620")), 1620.0);
        assert_eq!(super::count(&serde_json::json!("1.2M")), 1_200_000.0);
        assert_eq!(super::count(&serde_json::json!("")), 0.0);
    }
}
