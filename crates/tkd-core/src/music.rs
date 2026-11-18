use crate::proc::TreeExt;
use crate::{config::Channel, footage, media, state::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Uma trilha baixada para o canal, ja com a grade de batidas medida.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// caminho relativo a video/public/ — e o que vai no campo `audio` da spec
    pub file: String,
    pub source: String,
    pub title: String,
    pub duration: f64,
    /// batidas detectadas (segundos)
    pub beats: Vec<f64>,
    pub bpm: f64,
    /// trecho mais denso em batidas: onde o drop costuma estar
    pub drop_at: f64,
    /// referencia de onde a trilha saiu (a musica da propria ref, alinhada com a estrutura que o recreate copia)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
}

pub fn audio_dir(ch: &Channel) -> PathBuf {
    footage::public_dir().join(&ch.slug).join("audio")
}

pub fn manifest_path(ch: &Channel) -> PathBuf {
    ch.knowledge_dir().join("tracks.json")
}

pub fn load_tracks(ch: &Channel) -> Vec<Track> {
    crate::config::load_json(&manifest_path(ch))
}

pub fn save_tracks(ch: &Channel, t: &[Track]) -> Result<()> {
    crate::config::save_json(&manifest_path(ch), t)
}

pub fn digest(ch: &Channel) -> String {
    digest_for(ch, None)
}

/// Trilhas para o prompt do recreate. A da propria ref (`ref_id`) vem primeiro e marcada: e a musica em que os
/// cortes copiados foram feitos, entao com `musicStart` 0 as viradas caem na batida sem ajuste.
pub fn digest_for(ch: &Channel, ref_id: Option<&str>) -> String {
    let mut t = load_tracks(ch);
    if t.is_empty() {
        return "(sem trilha — o video vai sair mudo)".into();
    }
    let is_ref = |x: &Track| ref_id.is_some() && x.ref_id.as_deref() == ref_id;
    t.sort_by_key(|x| !is_ref(x));
    t.iter()
        .map(|t| {
            let mark = if is_ref(t) {
                " | MUSICA DA PROPRIA REF (use esta com musicStart 0: os cortes da ref ja estao na batida dela)"
            } else {
                ""
            };
            format!(
                "- {} | {:.0}s | {:.0} bpm | drop por volta de {:.1}s | \"{}\"{mark}",
                t.file, t.duration, t.bpm, t.drop_at, t.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Grade de batidas de um arquivo de audio: onsets + bpm estimado.
pub async fn beat_grid(file: &std::path::Path) -> Result<(Vec<f64>, f64)> {
    let a = media::audio_envelope(file).await?;
    let beats = a.hits;
    // bpm pela mediana do intervalo entre onsets
    let mut intervals: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    intervals.retain(|i| (0.2..=1.5).contains(i));
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let bpm = if intervals.is_empty() {
        0.0
    } else {
        60.0 / intervals[intervals.len() / 2]
    };
    Ok((beats, bpm))
}

/// Janela de 10s com mais batidas — serve de ponto de entrada do drop.
fn densest_window(beats: &[f64], win: f64) -> f64 {
    let mut top_pick = (0.0, 0usize);
    for b in beats {
        let n = beats.iter().filter(|x| **x >= *b && **x < *b + win).count();
        if n > top_pick.1 {
            top_pick = (*b, n);
        }
    }
    top_pick.0
}

/// Baixa uma trilha do YouTube (so audio) e cataloga a grade de batidas.
pub async fn fetch(ch: &Channel, store: &Store, query: &str) -> Result<Track> {
    let dir = audio_dir(ch);
    std::fs::create_dir_all(&dir)?;
    let stem = format!("{}-{}", ch.slug, chrono::Utc::now().format("%m%d%H%M%S"));
    let out_tpl = dir.join(format!("{stem}.%(ext)s"));

    // busca flat (so a pagina de resultados): 1o resultado com menos de 7 min
    let hit = footage::search(query, 5)
        .await?
        .into_iter()
        .find(|h| h.duration.is_some_and(|d| d > 0.0 && d < 420.0))
        .ok_or_else(|| anyhow::anyhow!("nenhuma trilha para \"{query}\""))?;
    let (url, title_text) = (hit.url(), hit.title.clone());
    let url = url.as_str();

    footage::ytdlp_public(&[
        url, "--no-playlist", "-x", "--audio-format", "mp3", "--audio-quality", "0",
        "-o", &out_tpl.to_string_lossy(), "--no-progress", "--quiet",
    ])
    .await
    .context("baixando audio")?;

    let mp3 = dir.join(format!("{stem}.mp3"));
    anyhow::ensure!(mp3.exists(), "download de audio nao gerou mp3");

    let probe = media::probe(&mp3).await?;
    let (beats, bpm) = beat_grid(&mp3).await?;
    let track = Track {
        file: mp3
            .strip_prefix(footage::public_dir())
            .unwrap_or(&mp3)
            .to_string_lossy()
            .replace('\\', "/"),
        source: url.to_string(),
        title: title_text.to_string(),
        duration: probe.duration,
        drop_at: densest_window(&beats, 10.0),
        beats,
        bpm,
        ref_id: None,
        artist: String::new(),
    };

    let mut tracks = load_tracks(ch);
    tracks.push(track.clone());
    save_tracks(ch, &tracks)?;
    store.log(&ch.slug, "music", "ok", &track.title)?;
    tracing::info!(
        "[{}] trilha: \"{}\" | {:.0}s | {:.0} bpm | {} batidas",
        ch.slug,
        track.title,
        track.duration,
        track.bpm,
        track.beats.len()
    );
    Ok(track)
}

/// Abaixo disso (dBFS medio) o stem de musica e silencio/vazamento: a ref nao tem musica propria.
const MIN_MUSIC_DB: f64 = -40.0;
/// Fala de personagem abaixo disso (fracao da duracao): a mixagem original serve inteira (mantem a voz da musica).
const MAX_SPEECH_FOR_MIX: f64 = 0.05;

/// Volume medio (dBFS) de um arquivo de audio, pelo volumedetect do ffmpeg.
async fn mean_volume(file: &Path) -> Option<f64> {
    let o = tokio::process::Command::new(media::ffmpeg_bin())
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(file)
        .args(["-af", "volumedetect", "-f", "null", "-"])
        .tree_output()
        .await
        .ok()?;
    let err = String::from_utf8_lossy(&o.stderr);
    err.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches("dB").trim().parse::<f64>().ok())
}

/// Qual arquivo da pasta stems/ vira a trilha: mixagem inteira se a ref quase nao tem fala de personagem,
/// senao a musica sem vozes (a fala da ref e de outro personagem e nao pode entrar no edit).
fn stem_choice(speech_ratio: f64) -> &'static str {
    if speech_ratio < MAX_SPEECH_FOR_MIX {
        "original.wav"
    } else {
        "music.wav"
    }
}

/// Garante no tracks.json a trilha da referencia `ref_id`, sem YouTube:
///  1. a musica da propria ref (stems do `rea-video decompose`): mesmo trecho e andamento em que os cortes
///     copiados foram feitos;
///  2. reserva: previa oficial (~30 s) da faixa que o Shazam identificou na ref (scripts/shazam_preview.py).
/// Nada disso gasta orcamento do YouTube. Sem nenhuma das duas: None (o agente usa outra trilha do canal).
pub async fn ensure_ref_track(ch: &Channel, ref_id: &str, ref_mp4: &Path, adir: &Path) -> Option<Track> {
    let mut tracks = load_tracks(ch);
    if let Some(t) = tracks
        .iter()
        .find(|t| t.ref_id.as_deref() == Some(ref_id) && footage::public_dir().join(&t.file).exists())
    {
        return Some(t.clone());
    }
    let meta: serde_json::Value = std::fs::read_to_string(ch.ref_meta_dir().join(format!("{ref_id}.json")))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let speech_ratio = meta["sound"]["speech"]["ratio"].as_f64().unwrap_or(1.0);
    let shazam = meta["sound"]["tracks"].as_array().and_then(|a| a.first()).cloned().unwrap_or_default();
    let (mut title, mut artist) =
        (shazam["title"].as_str().unwrap_or_default().to_string(), shazam["artist"].as_str().unwrap_or_default().to_string());

    let dir = audio_dir(ch);
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    // WAV, nao MP3: o render ja decodifica e o loudnorm codifica o AAC final; MP3 era uma geracao com perda a
    // mais e o atraso do encoder mexia no musicStart 0 (= batidas da ref)
    let mut dst = dir.join(format!("ref-{ref_id}.wav"));
    let mut source = format!("ref:{ref_id}");

    // 1. stems da ref (o decompose do recreate ja gera; se faltar, so a separacao)
    let stems = adir.join("stems");
    if !stems.join("music.wav").exists() {
        let st = tokio::process::Command::new(crate::recreate::venv_bin("rea-video"))
            .arg("stems")
            .arg(ref_mp4)
            .arg("-o")
            .arg(adir)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .current_dir(crate::config::root())
            .tree_output()
            .await;
        if !matches!(&st, Ok(o) if o.status.success()) {
            tracing::warn!("[{}] separacao da musica da ref {ref_id} falhou", ch.slug);
        }
    }
    let mut src = stems.join(stem_choice(speech_ratio));
    if !src.exists() {
        src = stems.join("music.wav");
    }
    let mut ok = false;
    if src.exists() {
        match mean_volume(&src).await {
            Some(db) if db >= MIN_MUSIC_DB => {
                ok = tokio::process::Command::new(media::ffmpeg_bin())
                    .args(["-v", "error", "-y", "-i"])
                    .arg(&src)
                    .args(["-c:a", "pcm_s16le"])
                    .arg(&dst)
                    .tree_status()
                    .await
                    .map(|s| s.success())
                    .unwrap_or(false)
                    && dst.exists();
                if ok {
                    tracing::info!(
                        "[{}] trilha da propria ref {ref_id} ({}, {db:.0} dB, fala {:.0}%)",
                        ch.slug,
                        src.file_name().unwrap_or_default().to_string_lossy(),
                        speech_ratio * 100.0
                    );
                }
            }
            Some(db) => tracing::info!("[{}] ref {ref_id} sem musica separavel ({db:.0} dB)", ch.slug),
            None => tracing::warn!("[{}] nao consegui medir o volume de {}", ch.slug, src.display()),
        }
    }

    // 2. previa do Shazam
    if !ok {
        let Some(key) = shazam["key"].as_str().filter(|k| !k.is_empty()) else {
            tracing::info!("[{}] ref {ref_id}: sem musica propria e sem faixa identificada pelo Shazam", ch.slug);
            return None;
        };
        let m4a = dir.join(format!("ref-{ref_id}.m4a"));
        let o = tokio::process::Command::new(crate::recreate::venv_bin("python"))
            .args(["-I", "-X", "utf8"])
            .arg(crate::config::root().join("scripts").join("shazam_preview.py"))
            .arg(key)
            .arg(&m4a)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .current_dir(crate::config::root())
            .tree_output()
            .await
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
        if v["ok"].as_bool() != Some(true) || !m4a.exists() {
            tracing::warn!("[{}] previa do Shazam ({key}): {}", ch.slug, v["error"].as_str().unwrap_or("falhou"));
            return None;
        }
        // a previa ja vem em AAC: fica como esta (reencodar em MP3 so perdia qualidade)
        dst = m4a;
        if let Some(t) = v["title"].as_str().filter(|t| !t.is_empty()) {
            title = t.to_string();
        }
        if let Some(a) = v["artist"].as_str().filter(|a| !a.is_empty()) {
            artist = a.to_string();
        }
        source = format!("shazam:{key}");
        tracing::info!("[{}] trilha da ref {ref_id}: previa do Shazam \"{title}\"", ch.slug);
    }

    let duration = media::probe(&dst).await.ok()?.duration;
    // batidas pelo librosa: o detector de onsets do media.rs acha 1-9 "batidas" em musica de edit (mixagem a
    // -6 LUFS, sem dinamica). Trilha da ref = mesmo tempo da ref, entao vale o measure.json que o recreate ja fez.
    let measured = if source.starts_with("ref:") { ref_beats(adir) } else { None };
    let (beats, bpm) = match measured {
        Some(b) => b,
        None => match librosa_beats(&dst).await {
            Some(b) => b,
            None => beat_grid(&dst).await.unwrap_or_default(),
        },
    };
    let drop_at = meta["sound"]["music"]["drop_s"]
        .as_f64()
        .filter(|_| source.starts_with("ref:"))
        .unwrap_or_else(|| densest_window(&beats, 10.0));
    let track = Track {
        file: dst.strip_prefix(footage::public_dir()).unwrap_or(&dst).to_string_lossy().replace('\\', "/"),
        source,
        title: if title.is_empty() { format!("trilha da ref {ref_id}") } else { title },
        duration,
        drop_at,
        beats,
        bpm,
        ref_id: Some(ref_id.to_string()),
        artist,
    };
    tracks.retain(|t| t.ref_id.as_deref() != Some(ref_id));
    tracks.push(track.clone());
    if let Err(e) = save_tracks(ch, &tracks) {
        tracing::warn!("[{}] tracks.json: {e:#}", ch.slug);
    }
    Some(track)
}

/// (batidas, bpm) do measure.json da ref (scripts/measure_ref.py, librosa).
fn ref_beats(adir: &Path) -> Option<(Vec<f64>, f64)> {
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(adir.join("measure.json")).ok()?).ok()?;
    let beats: Vec<f64> = m["music"]["beats"].as_array()?.iter().filter_map(|b| b.as_f64()).collect();
    let period = m["music"]["beat_s"].as_f64().filter(|p| *p > 0.0)?;
    (beats.len() >= 4).then_some((beats, 60.0 / period))
}

/// (batidas, bpm) pelo librosa (python do rea-video), para trilha sem measure.json.
async fn librosa_beats(file: &Path) -> Option<(Vec<f64>, f64)> {
    const CODE: &str = "import sys,json,numpy as np,librosa\n\
        y,sr=librosa.load(sys.argv[1],sr=22050,mono=True)\n\
        t,b=librosa.beat.beat_track(y=y,sr=sr,units='time')\n\
        print(json.dumps({'bpm':float(np.atleast_1d(t)[0]),'beats':[round(float(x),3) for x in b]}))";
    let o = tokio::process::Command::new(crate::recreate::venv_bin("python"))
        .args(["-I", "-X", "utf8", "-c", CODE])
        .arg(file)
        .tree_output()
        .await
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
    let beats: Vec<f64> = v["beats"].as_array()?.iter().filter_map(|b| b.as_f64()).collect();
    (beats.len() >= 4).then(|| (beats, v["bpm"].as_f64().unwrap_or(0.0)))
}

/// Batidas de uma trilha ja baixada, a partir do caminho que a spec usa.
pub fn beats_of(ch: &Channel, file: &str) -> Option<Track> {
    load_tracks(ch).into_iter().find(|t| t.file == file)
}

/// Encaixa as viradas de cena na batida da trilha.
/// O ajuste e pequeno de proposito: nenhuma cena pode crescer alem do
/// clipe que tem por tras, senao volta a congelar.
pub fn snap_to_beats(
    spec: &mut crate::spec::Spec,
    track: &Track,
    clip_len: &std::collections::HashMap<String, f64>,
) {
    if track.beats.len() < 4 {
        return;
    }
    let start_at = spec.audio_start.unwrap_or(0.0);
    // batidas em tempo de video (0 = comeco do video)
    let grade: Vec<f64> = track
        .beats
        .iter()
        .filter(|b| **b >= start_at)
        .map(|b| b - start_at)
        .collect();
    if grade.len() < 4 {
        return;
    }

    const TOLERANCE: f64 = 0.18;
    let mut cursor = 0.0;
    let mut fitted = 0;
    for s in spec.scenes.iter_mut() {
        let current_end = cursor + s.duration_s;
        // batida mais proxima do fim desta cena
        let goal = grade
            .iter()
            .min_by(|a, b| {
                (*a - current_end)
                    .abs()
                    .partial_cmp(&(*b - current_end).abs())
                    .unwrap()
            })
            .copied()
            .unwrap_or(current_end);

        let new_dur = goal - cursor;
        let max_dur = s
            .media
            .as_ref()
            .and_then(|m| clip_len.get(m).copied())
            .map(|total| total - s.media_start.unwrap_or(0.0))
            .unwrap_or(f64::INFINITY);

        if (new_dur - s.duration_s).abs() <= TOLERANCE && new_dur > 0.25 && new_dur <= max_dur {
            s.duration_s = new_dur;
            fitted += 1;
        }
        s.start_s = cursor;
        cursor += s.duration_s;
    }
    spec.duration_s = cursor;
    tracing::info!(
        "{} de {} viradas encaixadas na batida ({:.0} bpm)",
        fitted,
        spec.scenes.len(),
        track.bpm
    );
}

