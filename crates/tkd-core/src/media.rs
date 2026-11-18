use crate::proc::TreeExt;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Probe {
    pub duration: f64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

/// Um plano: trecho entre dois cortes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Shot {
    pub index: usize,
    pub start: f64,
    pub duration: f64,
    /// Movimento medio dentro do plano (0-1, derivado do scene score).
    pub motion: f64,
    /// Cores dominantes do plano.
    pub palette: Vec<String>,
    /// Frame representativo (PNG) extraido do meio do plano.
    pub frame: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Audio {
    /// Loudness integrado do arquivo inteiro (LUFS).
    pub lufs: Option<f64>,
    /// Envelope de energia: (segundo, RMS em dB).
    pub envelope: Vec<(f64, f64)>,
    /// Onsets — subidas bruscas de energia, onde a edicao costuma cortar.
    pub hits: Vec<f64>,
    /// Quantos cortes caem a menos de 0.15s de um onset (sincronia com a musica).
    pub cuts_on_beat: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Analysis {
    pub probe: Probe,
    /// Timestamps (s) das trocas de cena.
    pub cuts: Vec<f64>,
    /// Trocas de plano com o tipo de transicao (cut/dissolve).
    pub transitions: Vec<Transition>,
    /// Fracao do video sem movimento nenhum (indicador de carrossel de fotos).
    pub static_ratio: f64,
    /// Parece post de fotos em slide, nao video editado.
    pub is_slideshow: bool,
    pub avg_cut: f64,
    pub cuts_per_sec: f64,
    /// Paleta dominante do video inteiro.
    pub palette: Vec<String>,
    pub lufs: Option<f64>,
    /// Plano a plano, cobrindo o video inteiro.
    pub shots: Vec<Shot>,
    /// Curva de movimento amostrada a 10 fps: (segundo, score 0-1).
    pub motion_curve: Vec<(f64, f64)>,
    pub motion_avg: f64,
    pub motion_max: f64,
    pub audio: Audio,
    /// Mosaico com o video inteiro em uma imagem.
    pub contact_sheet: Option<String>,
    /// Frames representativos de cada plano.
    pub keyframes: Vec<String>,
}

/// Tarja preta embutida na fonte (letterbox/pillarbox), como filtro `crop=w:h:x:y` para vir ANTES do recorte no
/// formato do canal. None = sem tarja (ou nao deu para medir). Mede 3 trechos de 4 s (20%, 50%, 80%) com o
/// cropdetect e fica com a area MAIOR entre eles: cena escura num trecho nao pode virar "tarja".
/// 17/11: um clipe saiu com 11 linhas pretas em cima, sobra da tarja do video do YouTube.
pub async fn detect_bars(file: &Path, duration: f64) -> Option<String> {
    let probe = probe(file).await.ok()?;
    let (w, h) = (probe.width as i64, probe.height as i64);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut best: Option<(i64, i64, i64, i64)> = None;
    for frac in [0.2, 0.5, 0.8] {
        let at = (duration * frac).max(0.0);
        let o = Command::new(ffmpeg_bin())
            .args(["-hide_banner", "-nostats", "-ss", &format!("{at:.2}"), "-t", "4", "-i"])
            .arg(file)
            .args(["-vf", "cropdetect=limit=24:round=2:reset=0", "-an", "-f", "null", "-"])
            .tree_output()
            .await
            .ok()?;
        let err = String::from_utf8_lossy(&o.stderr);
        let Some(c) = err.lines().rev().find_map(|l| l.split("crop=").nth(1)).and_then(parse_crop) else { continue };
        // a maior area vista vale (um trecho com a tela toda e prova de que nao ha tarja ali)
        if best.is_none_or(|b| c.0 * c.1 > b.0 * b.1) {
            best = Some(c);
        }
    }
    let (cw, ch, cx, cy) = best?;
    is_bar((w, h), (cw, ch, cx, cy)).then(|| format!("crop={cw}:{ch}:{cx}:{cy}"))
}

/// O recorte do cropdetect e tarja de verdade? Cena escura engana o cropdetect (canal de edits: ceu noturno = "232 px em
/// cima"). Vale tarja simetrica (letterbox/pillarbox: os dois lados opostos, parecidos) ou fina (<= 5% do quadro,
/// ex. 12 px de sobra em cima). Faixa grande de um lado so = conteudo escuro, nao corta.
fn is_bar((w, h): (i64, i64), (cw, ch, cx, cy): (i64, i64, i64, i64)) -> bool {
    let (left, top, right, bottom) = (cx, cy, w - cw - cx, h - ch - cy);
    if [left, top, right, bottom].iter().all(|&c| c < 4) || cw * 10 < w * 6 || ch * 10 < h * 6 {
        return false;
    }
    let side_ok = |a: i64, b: i64, size: i64| {
        let (big, small) = (a.max(b), a.min(b));
        big < 4 || big * 20 <= size || (small >= 4 && big - small <= size / 50)
    };
    side_ok(top, bottom, h) && side_ok(left, right, w)
}

pub(crate) fn parse_crop(s: &str) -> Option<(i64, i64, i64, i64)> {
    let v: Vec<i64> = s.trim().split(':').take(4).filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() == 4 && v[0] > 0 && v[1] > 0).then(|| (v[0], v[1], v[2], v[3]))
}

/// Binario do ffmpeg: $TKD_FFMPEG ou `ffmpeg` do PATH. O projeto assume FFmpeg 9+ (swscale novo, mais rapido).
/// Mesmo arquivo em dois lugares sem copiar os bytes (hard link; outro disco = copia). Os mp4 nunca sao
/// editados no lugar (loudnorm escreve num temporario e renomeia), entao o link e seguro.
pub fn link_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(dst);
    std::fs::hard_link(src, dst).or_else(|_| std::fs::copy(src, dst).map(|_| ()))
}

pub fn ffmpeg_bin() -> String {
    std::env::var("TKD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

/// ffprobe ao lado do ffmpeg escolhido.
pub fn ffprobe_bin() -> String {
    let ff = ffmpeg_bin();
    let p = Path::new(&ff);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().replace("ffmpeg", "ffprobe"))
        .unwrap_or_else(|| "ffprobe".into());
    match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(d) => d.join(name).to_string_lossy().to_string(),
        None => name,
    }
}

/// Avisa (uma vez) se o ffmpeg em uso for anterior ao 9.
pub async fn check_ffmpeg() {
    static DONE_MARK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if DONE_MARK.set(()).is_err() {
        return;
    }
    match run(&ffmpeg_bin(), &["-hide_banner", "-version"]).await {
        Ok((out, _)) => {
            let v = out.split_whitespace().nth(2).unwrap_or("").to_string();
            let major: u32 = v.split(|c: char| !c.is_ascii_digit()).next().and_then(|m| m.parse().ok()).unwrap_or(0);
            if major < 9 {
                tracing::warn!("ffmpeg {v} em uso; o projeto espera FFmpeg 9+ (winget upgrade Gyan.FFmpeg ou TKD_FFMPEG)");
            } else {
                tracing::debug!("ffmpeg {v}");
            }
        }
        Err(e) => tracing::warn!("ffmpeg nao encontrado ({e}); instale o FFmpeg 9 ou defina TKD_FFMPEG"),
    }
}

/// stdout em bytes crus — obrigatorio para dados binarios (pixels da paleta).
/// Ler como String corromperia cada byte invalido em UTF-8 (EF BF BD).
async fn run_bytes(bin: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new(bin)
        .args(args)
        .tree_output()
        .await
        .with_context(|| format!("executando {bin}"))?;
    Ok(out.stdout)
}

pub(crate) async fn run(bin: &str, args: &[&str]) -> Result<(String, String)> {
    let out = Command::new(bin)
        .args(args)
        .tree_output()
        .await
        .with_context(|| format!("executando {bin}"))?;
    Ok((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub async fn probe(file: &Path) -> Result<Probe> {
    let path = file.to_string_lossy().to_string();
    let (stdout, _) = run(
        &ffprobe_bin(),
        &[
            "-v", "quiet", "-print_format", "json", "-show_streams", "-show_format", &path,
        ],
    )
    .await?;
    let v: serde_json::Value = serde_json::from_str(&stdout).context("ffprobe json")?;
    let empty = vec![];
    let streams = v["streams"].as_array().unwrap_or(&empty);
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");

    let fps = video
        .and_then(|s| s["avg_frame_rate"].as_str())
        .map(|r| {
            let (a, b) = r.split_once('/').unwrap_or((r, "1"));
            let a: f64 = a.parse().unwrap_or(0.0);
            let b: f64 = b.parse().unwrap_or(1.0);
            if b == 0.0 { 0.0 } else { a / b }
        })
        .unwrap_or(0.0);

    Ok(Probe {
        duration: v["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0.0),
        fps,
        width: video.and_then(|s| s["width"].as_u64()).unwrap_or(0) as u32,
        height: video.and_then(|s| s["height"].as_u64()).unwrap_or(0) as u32,
        has_audio,
    })
}

/// Curva de movimento: score de mudanca de cena a cada frame, 10 fps.
/// Usa o filtro `scdet` (o `select` nao exporta o score neste build do ffmpeg).
/// Uma unica passada cobre o video inteiro.
pub async fn motion_curve(file: &Path) -> Result<Vec<(f64, f64)>> {
    let path = file.to_string_lossy().to_string();
    let (stdout, stderr) = run(
        &ffmpeg_bin(),
        &[
            "-v", "info", "-i", &path,
            "-vf", "fps=10,scale=192:-2,scdet=threshold=0",
            "-an", "-f", "null", "-",
        ],
    )
    .await?;
    let text = format!("{stdout}
{stderr}");
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.split("lavfi.scd.score:").nth(1) else {
            continue;
        };
        let score: Option<f64> = rest.split(',').next().and_then(|v| v.trim().parse().ok());
        let time: Option<f64> = line
            .split("lavfi.scd.time:")
            .nth(1)
            .and_then(|v| v.trim().parse().ok());
        if let (Some(score), Some(time)) = (score, time) {
            // scdet entrega 0-100; normaliza para 0-1
            out.push((time, score / 100.0));
        }
    }
    Ok(out)
}

/// Uma troca de plano, com o tipo de transicao usado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub t: f64,
    /// "cut" = troca seca; "dissolve" = fusao/fade, score moderado e sustentado.
    pub kind: String,
    pub score: f64,
}

/// Deteccao adaptativa: edits com dissolve nunca passam de um limiar fixo alto,
/// entao o corte sai do proprio perfil estatistico da curva.
pub fn detect_transitions(curve: &[(f64, f64)]) -> Vec<Transition> {
    if curve.len() < 3 {
        return vec![];
    }
    let scores: Vec<f64> = curve.iter().map(|(_, s)| *s).collect();
    let media = scores.iter().sum::<f64>() / scores.len() as f64;
    let var = scores.iter().map(|s| (s - media).powi(2)).sum::<f64>() / scores.len() as f64;
    let deviation = var.sqrt();
    let thresh = (media + 2.0 * deviation).max(0.012);

    let mut out: Vec<Transition> = Vec::new();
    for i in 1..curve.len() - 1 {
        let (t, s) = curve[i];
        if s < thresh || s < curve[i - 1].1 || s < curve[i + 1].1 {
            continue;
        }
        if out.last().map(|l| t - l.t < 0.25).unwrap_or(false) {
            continue;
        }
        let neighborhood = (curve[i - 1].1 + curve[i + 1].1) / 2.0;
        let kind = if s > 0.10 && s > neighborhood * 2.5 { "cut" } else { "dissolve" };
        out.push(Transition { t, kind: kind.into(), score: s });
    }
    out
}

pub fn cuts_from_curve(curve: &[(f64, f64)], _threshold: f64) -> Vec<f64> {
    detect_transitions(curve).into_iter().map(|t| t.t).collect()
}

/// Post de carrossel de fotos: quase todo frame identico ao anterior,
/// com saltos secos entre as imagens. Nao serve de referencia de edicao.
pub fn looks_like_slideshow(curve: &[(f64, f64)], transitions: &[Transition]) -> bool {
    if curve.len() < 10 {
        return false;
    }
    let stalled = curve.iter().filter(|(_, s)| *s < 0.004).count() as f64 / curve.len() as f64;
    let only_plain_cuts = !transitions.is_empty()
        && transitions.iter().all(|t| t.kind == "cut")
        && transitions.len() <= 10;
    stalled > 0.9 && only_plain_cuts
}

/// Envelope de energia (RMS a 10 Hz) + onsets, e o LUFS integrado.
/// `astats` imprime no stdout; `ebur128` so e usado para o valor integrado.
pub async fn audio_envelope(file: &Path) -> Result<Audio> {
    let path = file.to_string_lossy().to_string();
    let (stdout, _) = run(
        &ffmpeg_bin(),
        &[
            "-v", "error", "-i", &path,
            "-af",
            "aresample=8000,asetnsamples=800,astats=metadata=1:reset=1,ametadata=print:key=lavfi.astats.Overall.RMS_level:file=-",
            "-f", "null", "-",
        ],
    )
    .await?;

    let mut envelope: Vec<(f64, f64)> = Vec::new();
    let mut t = None::<f64>;
    for line in stdout.lines() {
        if let Some(rest) = line.split("pts_time:").nth(1) {
            t = rest.trim().parse().ok();
        } else if let Some(rest) = line.split("lavfi.astats.Overall.RMS_level=").nth(1) {
            if let (Some(time), Ok(db)) = (t, rest.trim().parse::<f64>()) {
                if db.is_finite() && db > -90.0 {
                    envelope.push((time, db));
                }
            }
        }
    }

    // LUFS integrado do arquivo inteiro
    let (_, stderr) = run(
        &ffmpeg_bin(),
        &["-v", "info", "-i", &path, "-af", "ebur128", "-f", "null", "-"],
    )
    .await?;
    let lufs = stderr
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("I:").map(|v| v.trim().to_string()))
        .and_then(|v| v.split_whitespace().next().and_then(|n| n.parse::<f64>().ok()));

    // onset = subida brusca de energia (>3 dB) que tambem e pico local
    let mut hits: Vec<f64> = Vec::new();
    for i in 1..envelope.len().saturating_sub(1) {
        let (time, db) = envelope[i];
        let rise = db - envelope[i - 1].1;
        if rise > 3.0 && db >= envelope[i + 1].1 {
            if hits.last().map(|l| time - *l > 0.2).unwrap_or(true) {
                hits.push(time);
            }
        }
    }

    Ok(Audio { lufs, envelope, hits, cuts_on_beat: 0 })
}

/// Paleta dominante de um trecho (ou do arquivo inteiro, se `ss`/`t` forem None).
pub async fn palette_range(
    file: &Path,
    colors: u32,
    ss: Option<f64>,
    t: Option<f64>,
) -> Result<Vec<String>> {
    let path = file.to_string_lossy().to_string();
    let tag = format!(
        "{}-{}",
        file.file_stem().unwrap_or_default().to_string_lossy(),
        ss.unwrap_or(-1.0)
    );
    let tmp = std::env::temp_dir().join(format!("tkd-pal-{tag}.png"));
    let tmp_s = tmp.to_string_lossy().to_string();
    let filter =
        format!("fps=2,scale=160:-1:flags=lanczos,palettegen=max_colors={colors}:stats_mode=full");

    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    if let Some(ss) = ss {
        args.push("-ss".into());
        args.push(format!("{ss:.2}"));
    }
    if let Some(t) = t {
        args.push("-t".into());
        args.push(format!("{t:.2}"));
    }
    args.extend(["-i".into(), path, "-vf".into(), filter, tmp_s.clone()]);
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run(&ffmpeg_bin(), &refs).await?;
    if !tmp.exists() {
        return Ok(vec![]);
    }

    let bytes = run_bytes(
        &ffmpeg_bin(),
        &["-v", "error", "-i", &tmp_s, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
    )
    .await?;
    let mut hex = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for px in bytes.chunks_exact(3) {
        let h = format!("#{:02x}{:02x}{:02x}", px[0], px[1], px[2]);
        // palettegen preenche as sobras da paleta com verde puro
        if h == "#00ff00" {
            continue;
        }
        if seen.insert(h.clone()) {
            hex.push(h);
        }
        if hex.len() >= colors as usize {
            break;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(hex)
}

pub async fn palette(file: &Path, colors: u32) -> Result<Vec<String>> {
    palette_range(file, colors, None, None).await
}

/// Mosaico cobrindo o video inteiro: `cols`x`rows` frames em uma imagem.
pub async fn contact_sheet(file: &Path, out: &Path, cols: u32, rows: u32) -> Result<Option<PathBuf>> {
    let p = probe(file).await?;
    if p.duration <= 0.0 {
        return Ok(None);
    }
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    let total = (cols * rows) as f64;
    let rate = total / p.duration; // frames por segundo amostrados
    let path = file.to_string_lossy().to_string();
    let out_s = out.to_string_lossy().to_string();
    let filter = format!(
        "fps={rate:.4},scale=216:-2,drawtext=text='%{{eif\\:t*{rate:.4}\\:d}}':x=6:y=6:fontsize=18:fontcolor=white:box=1:boxcolor=black@0.6,tile={cols}x{rows}"
    );
    let (_, err) = run(
        &ffmpeg_bin(),
        &["-y", "-v", "error", "-i", &path, "-vf", &filter, "-frames:v", "1", &out_s],
    )
    .await?;
    if !out.exists() {
        // drawtext pode faltar em alguns builds: refaz sem o rotulo
        let filter = format!("fps={rate:.4},scale=216:-2,tile={cols}x{rows}");
        run(
            &ffmpeg_bin(),
            &["-y", "-v", "error", "-i", &path, "-vf", &filter, "-frames:v", "1", &out_s],
        )
        .await?;
        if !out.exists() {
            tracing::warn!("contact sheet falhou: {err}");
            return Ok(None);
        }
    }
    Ok(Some(out.to_path_buf()))
}

/// Um frame do meio de cada plano.
pub async fn frame_at(file: &Path, t: f64, dst: &Path, width: u32) -> Result<Option<PathBuf>> {
    std::fs::create_dir_all(dst.parent().unwrap_or(Path::new(".")))?;
    let path = file.to_string_lossy().to_string();
    let dst_s = dst.to_string_lossy().to_string();
    let scale = format!("scale={width}:-2");
    run(
        &ffmpeg_bin(),
        &["-y", "-v", "error", "-ss", &format!("{t:.2}"), "-i", &path,
          "-frames:v", "1", "-vf", &scale, &dst_s],
    )
    .await?;
    Ok(dst.exists().then(|| dst.to_path_buf()))
}

/// Extrai `count` frames espalhados pelo video (usado no review do proprio render).
pub async fn frames(file: &Path, out_dir: &Path, count: u32) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let p = probe(file).await?;
    let stem = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let mut out = Vec::new();
    for i in 0..count {
        let t = p.duration * (i as f64 + 0.5) / count as f64;
        let dst = out_dir.join(format!("{stem}-f{i}.png"));
        if let Some(p) = frame_at(file, t, &dst, 360).await? {
            out.push(p);
        }
    }
    Ok(out)
}

/// Analise completa: cobre o video inteiro — plano a plano, movimento, audio e cor.
pub async fn analyze(file: &Path, assets_dir: &Path) -> Result<Analysis> {
    let probe = probe(file).await?;
    let stem = file.file_stem().unwrap_or_default().to_string_lossy().to_string();

    let motion_curve = motion_curve(file).await.unwrap_or_default();
    let transitions = detect_transitions(&motion_curve);
    let cuts: Vec<f64> = transitions.iter().map(|t| t.t).collect();
    let static_ratio = if motion_curve.is_empty() {
        0.0
    } else {
        motion_curve.iter().filter(|(_, s)| *s < 0.004).count() as f64 / motion_curve.len() as f64
    };
    let is_slideshow = looks_like_slideshow(&motion_curve, &transitions);
    let motion_avg = if motion_curve.is_empty() {
        0.0
    } else {
        motion_curve.iter().map(|(_, s)| s).sum::<f64>() / motion_curve.len() as f64
    };
    let motion_max = motion_curve.iter().map(|(_, s)| *s).fold(0.0, f64::max);

    let mut audio = if probe.has_audio {
        audio_envelope(file).await.unwrap_or_default()
    } else {
        Audio::default()
    };
    audio.cuts_on_beat = cuts
        .iter()
        .filter(|c| audio.hits.iter().any(|h| (h - *c).abs() < 0.15))
        .count();

    // planos: fronteiras = inicio, cortes, fim
    let mut bounds = vec![0.0];
    bounds.extend(cuts.iter().copied());
    bounds.push(probe.duration);
    bounds.dedup_by(|a, b| (*a - *b).abs() < 0.2);

    let frames_dir = assets_dir.join(&stem);
    let mut shots = Vec::new();
    let mut keyframes = Vec::new();
    for i in 0..bounds.len().saturating_sub(1) {
        let (start, end) = (bounds[i], bounds[i + 1]);
        let duration = end - start;
        if duration <= 0.05 {
            continue;
        }
        let mid = start + duration / 2.0;
        let motion = {
            let win: Vec<f64> = motion_curve
                .iter()
                .filter(|(t, _)| *t >= start && *t < end)
                .map(|(_, s)| *s)
                .collect();
            if win.is_empty() {
                0.0
            } else {
                win.iter().sum::<f64>() / win.len() as f64
            }
        };
        // todos os planos ganham frame e paleta — cobertura integral do video
        let dst = frames_dir.join(format!("shot{:02}.png", i));
        let frame = frame_at(file, mid, &dst, 540).await.ok().flatten();
        let palette = palette_range(file, 4, Some(start), Some(duration.min(3.0)))
            .await
            .unwrap_or_default();
        if let Some(f) = &frame {
            keyframes.push(f.to_string_lossy().to_string());
        }
        shots.push(Shot {
            index: i,
            start,
            duration,
            motion,
            palette,
            frame: frame.map(|p| p.to_string_lossy().to_string()),
        });
    }

    let sheet = contact_sheet(file, &frames_dir.join("sheet.jpg"), 6, 6)
        .await
        .unwrap_or(None);

    let n = cuts.len().max(1) as f64;
    Ok(Analysis {
        avg_cut: if probe.duration > 0.0 { probe.duration / n } else { 0.0 },
        cuts_per_sec: if probe.duration > 0.0 { cuts.len() as f64 / probe.duration } else { 0.0 },
        palette: palette(file, 6).await.unwrap_or_default(),
        lufs: audio.lufs,
        cuts,
        transitions,
        static_ratio,
        is_slideshow,
        shots,
        motion_curve,
        motion_avg,
        motion_max,
        audio,
        contact_sheet: sheet.map(|p| p.to_string_lossy().to_string()),
        keyframes,
        probe,
    })
}

#[cfg(test)]
mod bars_tests {
    use super::*;

    #[test]
    fn crop_line() {
        assert_eq!(parse_crop("1080:1064:0:12 "), Some((1080, 1064, 0, 12)));
        assert_eq!(parse_crop("x:1:0:0"), None);
    }

    #[test]
    fn bar_or_dark_scene() {
        let sq = (1080, 1080);
        // 12 px de sobra em cima (clipes 185707-f0x do canal de edits): tarja fina
        assert!(is_bar(sq, (1080, 1068, 0, 12)));
        // ceu noturno do 173224-03: 232 px so em cima = cena escura
        assert!(!is_bar(sq, (1080, 848, 0, 232)));
        // letterbox de filme (2.39:1) dentro de uma fonte 1920x1080
        assert!(is_bar((1920, 1080), (1920, 800, 0, 140)));
        // nada a cortar
        assert!(!is_bar(sq, (1080, 1080, 0, 0)));
        // pillarbox de 1920x1080 com 4:3 dentro
        assert!(is_bar((1920, 1080), (1440, 1080, 240, 0)));
    }
}
