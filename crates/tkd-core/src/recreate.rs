//! `tkd recreate`: referencia + roteiro novo + banco de clipes -> video com a mesma estrutura/acabamento.
//!
//! Etapas (cada uma grava em analysis/<canal>/recreate/<run>/):
//!  1. rea-video decompose + scripts/measure_ref.py da ref (reaproveita se ja existir)
//!  2. agente (agents/recreate.md) escreve plan + voice  -> agent.json
//!  3. validacao do plano (caminhos, cobertura, janela do clipe)
//!  4. scripts/autograde.py calibra a cor de cada plano pela ref        -> plan.json
//!  5. scripts/voice_lines.py (ElevenLabs, se ELEVENLABS_API_KEY)        -> video/public/<canal>/recreate/<run>/voice.*
//!  6. scripts/assemble_plan.py junta tudo nas props do Remotion          -> props.json
//!  7. npx remotion render recreate                                       -> out/recreate/<run>.mp4
//!  8. scripts/compare.sh ref x remake (sbs, pares, sheets, rea compare) -> compare/
use crate::proc::TreeExt;
use crate::{agent, config::Channel, footage, music};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub struct Opts {
    pub ref_: String,
    pub script: String,
    pub end_s: Option<f64>,
    pub voice_id: String,
    pub plan: Option<PathBuf>,
    pub compare: bool,
    /// auto | clip (falas reais do footage) | tts (ElevenLabs) | none
    pub voice: String,
    /// personagem do edit: so clipes/falas dele
    pub character: Option<String>,
    /// preset de movimento (high|medium|low|none); None = o agente decide pelo guia (padrao medium)
    pub motion: Option<String>,
    /// idioma do edit (legenda): en | pt
    pub lang: String,
}

fn root() -> PathBuf {
    crate::config::root()
}

pub fn venv_bin(name: &str) -> PathBuf {
    let v = root().join("tools").join("rea-video").join(".venv");
    if cfg!(windows) {
        v.join("Scripts").join(format!("{name}.exe"))
    } else {
        v.join("bin").join(name)
    }
}

async fn run(label: &str, cmd: &mut Command) -> Result<()> {
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8").current_dir(root());
    tracing::info!("[recreate] {label}");
    let st = cmd.tree_status().await.with_context(|| format!("executando {label}"))?;
    anyhow::ensure!(st.success(), "{label} falhou ({st})");
    Ok(())
}

async fn python(label: &str, args: &[&str]) -> Result<()> {
    let mut c = Command::new(venv_bin("python"));
    c.args(["-I", "-X", "utf8"]).args(args);
    run(label, &mut c).await
}

/// No Windows `bash` no PATH costuma ser o do WSL; os scripts precisam do Git Bash.
fn bash() -> PathBuf {
    if let Ok(b) = std::env::var("TKD_BASH") {
        return PathBuf::from(b);
    }
    if cfg!(windows) {
        for p in [r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files (x86)\Git\bin\bash.exe"] {
            if Path::new(p).exists() {
                return PathBuf::from(p);
            }
        }
    }
    PathBuf::from("bash")
}

/// Motivo de as props renderizarem um video sem som, ou `None` se algo toca.
/// Toca: trilha, voz (TTS), voiceClip resolvido (from/to) ou plano com volume > 0.
fn silent_reason(props: &Value) -> Option<String> {
    let audio = &props["audio"];
    let has = |k: &str| audio[k].as_str().is_some_and(|s| !s.is_empty());
    let empty = Vec::new();
    let clips = props["voiceClips"].as_array().unwrap_or(&empty);
    let resolved = clips.iter().any(|v| v.get("from").is_some() && v.get("to").is_some());
    let shot_sound = props["shots"].as_array().unwrap_or(&empty).iter().any(|s| s["volume"].as_f64().unwrap_or(0.0) > 0.0);
    if has("music") || has("voice") || resolved || shot_sound {
        return None;
    }
    Some(if clips.is_empty() {
        "sem trilha no canal (knowledge/tracks.json) e sem fala no plano".into()
    } else {
        format!("{} fala(s) no plano sem from/to: clip_voice.py nao rodou; e sem trilha", clips.len())
    })
}

fn rel(p: &Path) -> String {
    p.strip_prefix(root()).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Resolve a ref: caminho existente ou id em channels/<canal>/refs/<id>.mp4.
fn resolve_ref(ch: &Channel, r: &str) -> Result<(PathBuf, String)> {
    let p = PathBuf::from(r);
    if p.exists() {
        let id = p.file_stem().unwrap().to_string_lossy().to_string();
        return Ok((p, id));
    }
    let p = ch.refs_dir().join(format!("{r}.mp4"));
    anyhow::ensure!(p.exists(), "ref nao encontrada: {r} (nem {})", p.display());
    Ok((p, r.to_string()))
}

/// Contact sheet da ref (10x6 quadros da ref inteira). Nome novo: as antigas (`sheet.jpg`) eram so 30 s e achatadas.
pub fn sheet_path(adir: &Path) -> PathBuf {
    adir.join("sheet_full.jpg")
}

/// Etapa 1: analise da ref (idempotente).
pub async fn analyze_ref(ch: &Channel, ref_mp4: &Path, id: &str) -> Result<PathBuf> {
    let dir = root().join("analysis").join(&ch.slug).join(id);
    if !dir.join("breakdown.json").exists() {
        let mut c = Command::new(venv_bin("rea-video"));
        c.args(["decompose", &rel(ref_mp4), "-o", &rel(&dir), "--lang", "en", "--no-vlm"]);
        run("rea-video decompose", &mut c).await?;
    }
    let m = dir.join("measure.json");
    if !m.exists() {
        python(
            "measure_ref",
            &["scripts/measure_ref.py", &rel(ref_mp4), &rel(&m), "--rea", &rel(&dir)],
        )
        .await?;
    }
    let sheet = sheet_path(&dir);
    if !sheet.exists() {
        // 60 quadros espalhados pela ref inteira (2 fps fixos cobriam so os 30 s iniciais de refs de ate 45 s)
        // e sem achatar ref nao quadrada
        let dur = crate::media::probe(ref_mp4).await.map(|p| p.duration).unwrap_or(30.0).max(1.0);
        let fps = (60.0 / dur).max(2.0);
        let vf = format!("fps={fps:.4},scale=180:-2,tile=10x6");
        let _ = Command::new(crate::media::ffmpeg_bin())
            .args(["-v", "error", "-y", "-i", &rel(ref_mp4), "-vf", &vf, "-frames:v", "1", &rel(&sheet)])
            .current_dir(root())
            .tree_status()
            .await;
    }
    Ok(dir)
}

fn guides(ch: &Channel) -> String {
    let k = ch.knowledge_dir();
    let mut out = String::new();
    for f in ["STYLE_GUIDE.md", "EDITING_RULES.md", "AUDIO_GUIDE.md"] {
        let t = read(&k.join(f));
        if !t.is_empty() {
            out.push_str(&format!("### {f}\n{t}\n"));
        }
    }
    if out.is_empty() {
        out = format!("### style-card.md (canal sem guias detalhados)\n{}", read(&ch.style_card()));
    }
    out
}

/// Trechos (s) em que a ref tem fala, do ASR do measure. Segmento de mais de 5 s e letra de musica alucinada
/// pelo ASR (canal de edits, 18/11: "Let's see dramas." em 7.3 s), nao fala. Falas a menos de 0.6 s viram um trecho so.
fn ref_speech_windows(measure: &Value, end_s: f64) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = measure["voice"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|l| Some((l["start"].as_f64()?, l["end"].as_f64()?.min(end_s))))
        .filter(|(a, b)| b - a > 0.3 && b - a <= 5.0)
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = vec![];
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a - l.1 < 0.6 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn fmt_windows(w: &[(f64, f64)]) -> String {
    w.iter().map(|(a, b)| format!("{a:.1}-{b:.1}s")).collect::<Vec<_>>().join(", ")
}

fn clean_word(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()
}

/// Erros do plano que o codigo confere contra dados reais (catalogo por frame e falas transcritas) e devolve ao
/// agente para corrigir. Nada aqui e regra de estilo: so o que da para provar errado.
/// Um mesmo clipe nao pode passar disso dos planos (o edit vira loop do mesmo plano).
const MAX_CLIP_SHARE: f64 = 0.25;
/// Clipes distintos que um edit precisa (ou todos os do personagem, se o banco tiver menos).
pub const MIN_DISTINCT_CLIPS: usize = 8;

/// Variedade da montagem. Em 18/11 tres edits seguidos foram reprovados por loop: 87-90 planos tirados de
/// 4-5 clipes, um deles repetido 38 vezes. `want` = clipes distintos exigidos (0 = nao confere).
fn check_variety(shots: &[Value], want: usize) -> Vec<String> {
    let mut errs = vec![];
    if want == 0 || shots.len() < 4 {
        return errs;
    }
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for s in shots {
        *uses.entry(s["src"].as_str().unwrap_or_default()).or_default() += 1;
    }
    let want = want.min(shots.len());
    if uses.len() < want {
        errs.push(format!(
            "variedade: {} planos com so {} clipes diferentes; use pelo menos {want} clipes distintos do personagem              (o catalogo tem mais) e espalhe os cortes entre eles",
            shots.len(),
            uses.len()
        ));
    }
    let cap = ((shots.len() as f64 * MAX_CLIP_SHARE).ceil() as usize).max(3);
    let mut heavy: Vec<(&str, usize)> = uses.into_iter().filter(|(_, n)| *n > cap).collect();
    heavy.sort_by(|a, b| b.1.cmp(&a.1));
    for (src, n) in heavy {
        errs.push(format!(
            "variedade: {src} aparece em {n} de {} planos (maximo {cap}); troque parte desses cortes por outros clipes",
            shots.len()
        ));
    }
    errs
}

fn check_plan(
    out: &Value,
    bank: &[footage::Clip],
    character: Option<&str>,
    fps: f64,
    ref_speech: &[(f64, f64)],
    min_distinct: usize,
) -> Vec<String> {
    let mut errs = vec![];
    let plan = &out["plan"];
    let clip = |src: &str| bank.iter().find(|c| c.file == src);
    let shots = plan["shots"].as_array().cloned().unwrap_or_default();
    errs.extend(check_variety(&shots, min_distinct));
    // trecho do clipe (s) que o plano mostra entre os frames [a, b) do edit
    let window = |s: &Value, a: f64, b: f64| {
        let (from, rate) = (s["from"].as_f64().unwrap_or(0.0), s["rate"].as_f64().unwrap_or(1.0));
        let st = s["startAt"].as_f64().unwrap_or(0.0);
        if s["freeze"].as_bool().unwrap_or(false) {
            return (st, st + 0.05);
        }
        (st + (a - from) / fps * rate, st + (b - from) / fps * rate)
    };
    // trechos (s) em que o plano tem fala, para comparar com a ref
    let mut voiced: Vec<(f64, f64)> = vec![];
    for (i, s) in shots.iter().enumerate() {
        let src = s["src"].as_str().unwrap_or_default();
        // antes passava calado aqui e o recreate inteiro morria depois no `validate`, sem o agente poder corrigir
        let Some(c) = clip(src) else {
            errs.push(format!("shot {i}: clipe inexistente {src:?}; use um `file` exatamente como esta no catalogo"));
            continue;
        };
        if !c.usable || c.tags.iter().any(|t| t == "so-audio" || t == footage::BURNED_SUBS_TAG) {
            errs.push(format!(
                "shot {i}: {src} esta marcado como inutilizavel para imagem ({}); use outro clipe (a fala dele so vale como voiceClip)",
                if c.tags.iter().any(|t| t == footage::BURNED_SUBS_TAG) { "legenda queimada" } else { "imagem ruim / so audio" }
            ));
        }
        let (a, b) = window(s, s["from"].as_f64().unwrap_or(0.0), s["to"].as_f64().unwrap_or(0.0));
        if let Some(name) = character {
            let others = c.others_in(a, b, name);
            if !others.is_empty() {
                errs.push(format!(
                    "shot {i}: {src} mostra {} entre {a:.2}-{b:.2}s (edit so de {name}); troque o clipe ou o startAt para um trecho sem PROIBIDO",
                    others.join(", ")
                ));
            }
        }
        if let Some(sp) = s.get("speech") {
            if let Some(line) = check_speech(&mut errs, &format!("shot {i}"), c, sp, character) {
                let (from, rate) = (s["from"].as_f64().unwrap_or(0.0), s["rate"].as_f64().unwrap_or(1.0).max(0.05));
                let st = s["startAt"].as_f64().unwrap_or(0.0);
                let to_s = |t: f64| from / fps + (t - st) / rate;
                voiced.push((to_s(line["start"].as_f64().unwrap_or(0.0)), to_s(line["end"].as_f64().unwrap_or(0.0))));
            }
        }
    }
    for (j, vc) in plan["voiceClips"].as_array().cloned().unwrap_or_default().iter().enumerate() {
        let src = vc["src"].as_str().unwrap_or_default();
        let Some(c) = clip(src) else {
            errs.push(format!("voiceClips[{j}]: clipe inexistente {src}"));
            continue;
        };
        let Some(line) = check_speech(&mut errs, &format!("voiceClips[{j}]"), c, vc, character) else { continue };
        let dur = line["end"].as_f64().unwrap_or(0.0) - line["start"].as_f64().unwrap_or(0.0);
        let (va, vb) = (vc["at"].as_f64().unwrap_or(0.0), vc["at"].as_f64().unwrap_or(0.0) + dur * fps);
        voiced.push((va / fps, vb / fps));
        for (i, s) in shots.iter().enumerate() {
            let (from, to) = (s["from"].as_f64().unwrap_or(0.0), s["to"].as_f64().unwrap_or(0.0));
            let (a, b) = (from.max(va), to.min(vb));
            if b - a < 2.0 {
                continue;
            }
            let Some(sc) = clip(s["src"].as_str().unwrap_or_default()) else { continue };
            let (ta, tb) = window(s, a, b);
            if let Some(who) = sc.face_in(ta, tb) {
                errs.push(format!(
                    "voiceClips[{j}] (fala so-audio, frames {va:.0}-{vb:.0}) toca sobre o shot {i}, que mostra o rosto de {who}                      ({ta:.2}-{tb:.2}s do clipe): parece dublagem errada. Troque o plano por um trecho 'sem rosto',                      ou use a fala no proprio plano (shots[i].speech)"
                ));
            }
        }
    }
    // a recriacao copia a ref: onde a ref tem fala, o edit tambem tem (Toji 18/11: so a 1a metade tinha voz e o v4
    // deu narrativa 5; a ref repete a fala na 2a metade)
    for (a, b) in ref_speech {
        let covered: f64 = voiced.iter().map(|(x, y)| (y.min(*b) - x.max(*a)).max(0.0)).sum();
        if covered < (b - a) * 0.5 {
            errs.push(format!(
                "a ref tem fala em {a:.1}-{b:.1}s (frames {:.0}-{:.0}) e o plano cobre so {:.0}% desse trecho: ponha uma fala                  real ali (voiceClips ou shots[i].speech), sem repetir a mesma fala",
                a * fps, b * fps, covered / (b - a) * 100.0
            ));
        }
    }
    errs
}

/// Fala `line` existe no clipe, e do personagem (falante do catalogo) e as palavras das legendas existem na fala
/// (ou na traducao). Devolve a fala.
fn check_speech(errs: &mut Vec<String>, at: &str, c: &footage::Clip, sp: &Value, character: Option<&str>) -> Option<Value> {
    let k = sp["line"].as_u64().unwrap_or(0) as usize;
    let Some(line) = c.speech.get(k).cloned() else {
        errs.push(format!("{at}: {} nao tem fala {k} (tem {})", c.file, c.speech.len()));
        return None;
    };
    if let (Some(name), Some(who)) = (character, line["speaker"].as_str()) {
        if !footage::unknown_person(who) && !footage::same_person(who, name) {
            errs.push(format!("{at}: a fala {k} de {} (\"{}\") e de {who}, nao de {name}; use so falas de {name}",
                c.file, line["text"].as_str().unwrap_or("")));
        }
    }
    let words: Vec<String> = match sp["translation"].as_str().filter(|t| !t.trim().is_empty()) {
        Some(t) => t.split_whitespace().map(clean_word).filter(|w| !w.is_empty()).collect(),
        None => line["words"].as_array().cloned().unwrap_or_default().iter()
            .map(|w| clean_word(w["w"].as_str().unwrap_or(""))).filter(|w| !w.is_empty()).collect(),
    };
    for cap in sp["captions"].as_array().cloned().unwrap_or_default() {
        for w in cap["words"].as_array().cloned().unwrap_or_default() {
            let ok = match &w {
                Value::Number(n) => (n.as_u64().unwrap_or(u64::MAX) as usize) < words.len(),
                Value::String(t) => words.contains(&clean_word(t)),
                _ => false,
            };
            if !ok {
                errs.push(format!("{at}: palavra {w} da legenda nao existe na fala (palavras: {})", words.join(" ")));
            }
        }
    }
    Some(line)
}

/// Etapa 3: corrige o que da para corrigir e falha no resto.
fn validate(plan: &mut Value, ch: &Channel, total: i64) -> Result<Vec<String>> {
    let mut warn = vec![];
    let bank: HashMap<String, f64> = footage::load_bank(ch).into_iter().map(|c| (c.file, c.duration)).collect();
    let fps = plan["fps"].as_f64().unwrap_or(30.0);
    let shots = plan["shots"].as_array_mut().ok_or_else(|| anyhow!("plano sem shots"))?;
    anyhow::ensure!(!shots.is_empty(), "plano com 0 shots");
    let mut cursor = 0i64;
    for (i, s) in shots.iter_mut().enumerate() {
        let src = s["src"].as_str().unwrap_or_default().to_string();
        anyhow::ensure!(footage::public_dir().join(&src).exists(), "shot {i}: clipe inexistente {src}");
        let from = s["from"].as_i64().unwrap_or(cursor);
        if from != cursor {
            warn.push(format!("shot {i}: from {from} -> {cursor} (buraco/sobreposicao)"));
            s["from"] = json!(cursor);
        }
        let to = s["to"].as_i64().unwrap_or(cursor + 1).max(cursor + 1);
        s["to"] = json!(to);
        cursor = to;
        if s.get("speech").is_some() {
            if (s["rate"].as_f64().unwrap_or(1.0) - 1.0).abs() > 1e-3 {
                warn.push(format!("shot {i}: plano com fala -> rate 1 (velocidade muda o tom da voz)"));
                s["rate"] = json!(1.0);
            }
            if s["volume"].as_f64().unwrap_or(0.0) <= 0.0 {
                s["volume"] = json!(1.0);
            }
            s["freeze"] = json!(false);
        }
        if let Some(dur) = bank.get(&src) {
            // margem de 2 frames: a duracao do banco e a pedida no corte, o arquivo pode ter 1 frame a menos
            let dur = (dur - 2.0 / fps).max(0.05);
            let rate = s["rate"].as_f64().unwrap_or(1.0);
            let freeze = s["freeze"].as_bool().unwrap_or(false);
            let need = if freeze { 0.05 } else { (to - from) as f64 / fps * rate };
            let start = s["startAt"].as_f64().unwrap_or(0.0);
            if start + need > dur {
                let ns = (dur - need).max(0.0);
                warn.push(format!("shot {i}: startAt {start:.2} -> {ns:.2} (clipe tem {dur:.2}s)"));
                s["startAt"] = json!((ns * 100.0).floor() / 100.0);
                // nem do comeco o clipe cobre o plano: desacelera para caber (sem fala) em vez de acabar em preto
                if need > dur && !freeze {
                    if s.get("speech").is_none() {
                        let nr = ((dur / ((to - from) as f64 / fps)) * 100.0).floor() / 100.0;
                        warn.push(format!("shot {i}: clipe de {dur:.2}s nao cobre {need:.2}s -> rate {rate:.2} -> {nr:.2}"));
                        s["rate"] = json!(nr.max(0.05));
                    } else {
                        warn.push(format!("shot {i}: plano com fala maior que o clipe ({need:.2}s > {dur:.2}s): o QA vai conferir o fim"));
                    }
                }
            }
        }
    }
    if cursor != total {
        warn.push(format!("ultimo shot termina em {cursor}, janela e {total}: ajustado"));
        let last = shots.last_mut().unwrap();
        last["to"] = json!(total);
    }
    Ok(warn)
}

/// Loudness do mp4 no alvo, em 2 passadas (o modo dinamico de 1 passada erra 2-3 LU em video curto / musica ja
/// comprimida). Troca o arquivo no lugar; false = ffmpeg falhou e o mp4 ficou como estava.
async fn loudnorm(out: &Path, lufs: f64) -> Result<bool> {
    let tmp = out.with_extension("norm.mp4");
    // task cancelada no meio (app fechado, timeout): o temporario some junto
    let _cleanup = crate::proc::RemoveOnDrop(tmp.clone());
    let mut af = format!("loudnorm=I={lufs}:TP=-1.0:LRA=11");
    if let Ok(m) = Command::new(crate::media::ffmpeg_bin())
        .args(["-hide_banner", "-i", &out.to_string_lossy(), "-vn", "-af",
               &format!("{af}:print_format=json"), "-f", "null", "-"])
        .tree_output()
        .await
    {
        let err = String::from_utf8_lossy(&m.stderr);
        if let Some(j) = err.rfind('{').and_then(|i| serde_json::from_str::<Value>(&err[i..err.rfind('}').unwrap_or(err.len() - 1) + 1]).ok()) {
            let g = |k: &str| j[k].as_str().unwrap_or("0").to_string();
            af = format!(
                "{af}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
                g("input_i"), g("input_tp"), g("input_lra"), g("input_thresh"), g("target_offset")
            );
        }
    }
    let ok = Command::new(crate::media::ffmpeg_bin())
        .args(["-v", "error", "-y", "-i", &out.to_string_lossy(), "-c:v", "copy",
               // mesmo bitrate do render (320k): 192k aqui jogava fora o que o Remotion entregou
               "-af", &af, "-ar", "48000", "-c:a", "aac", "-b:a", "320k",
               &tmp.to_string_lossy()])
        .tree_status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && tmp.exists() {
        std::fs::rename(&tmp, out)?;
        return Ok(true);
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(false)
}

/// Plano sem `audio.music` ganha a trilha da propria ref com `musicStart` 0. Devolve se mudou.
fn fill_music(plan: &mut Value, file: &str) -> bool {
    let has = plan["audio"]["music"].as_str().is_some_and(|m| !m.is_empty());
    if has {
        return false;
    }
    if !plan["audio"].is_object() {
        plan["audio"] = json!({});
    }
    plan["audio"]["music"] = json!(file);
    plan["audio"]["musicStart"] = json!(0);
    true
}

/// Erros do qa.json (regras de docs/QUALITY_RULES.md). Vazio = passou.
fn qa_errors(qa: &Value) -> Vec<String> {
    qa["issues"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter(|i| i["level"].as_str() == Some("erro"))
                .map(|i| {
                    let w = i["where"].as_str().unwrap_or("");
                    let what = i["what"].as_str().unwrap_or("?");
                    if w.is_empty() { what.to_string() } else { format!("{what} ({w})") }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// So loudness fora do alvo: da para corrigir com outra passada do loudnorm sem renderizar de novo.
fn only_loudness(errs: &[String]) -> bool {
    !errs.is_empty() && errs.iter().all(|e| e.starts_with("loudness"))
}

/// Secoes do report.md do rea-video que nao servem pro plano (prompts de IA, codigo, duvidas):
/// ~20 KB a menos relidos em todo turno do agente.
const REPORT_SKIP: [&str; 6] = ["AI generation", "Code build", "Open questions", "Metadata clues", "Notes", "Shoot"];

/// Colunas da tabela de planos que sao palpite de "como foi feito" (IA/3D/remake): nada disso entra no plano.
const REPORT_DROP_COLUMNS: [&str; 3] = ["Made with", "Material", "Remake"];

/// Tira do apendice "Shot by shot" os blocos "How it was made..." e as linhas "- Remake:" (palpites de
/// ferramenta, ~2/3 do apendice) e as colunas de `REPORT_DROP_COLUMNS` das tabelas. Fica o que se ve na tela.
fn drop_inferences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_inference = false;
    let mut drop_cols: Vec<usize> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("**How it was made") {
            in_inference = true;
            continue;
        }
        if in_inference {
            if t.starts_with("</details>") {
                in_inference = false;
            } else {
                continue;
            }
        }
        if t.starts_with("- Remake:") || t.starts_with("Each shot has two parts") {
            continue;
        }
        if t.starts_with('|') {
            let cells: Vec<&str> = t.trim_matches('|').split('|').collect();
            if drop_cols.is_empty() && cells.iter().any(|c| REPORT_DROP_COLUMNS.iter().any(|k| c.trim().starts_with(k))) {
                drop_cols = cells
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| REPORT_DROP_COLUMNS.iter().any(|k| c.trim().starts_with(k)))
                    .map(|(i, _)| i)
                    .collect();
            }
            if !drop_cols.is_empty() {
                let kept: Vec<&str> = cells.iter().enumerate().filter(|(i, _)| !drop_cols.contains(i)).map(|(_, c)| *c).collect();
                out.push('|');
                out.push_str(&kept.join("|"));
                out.push_str("|\n");
                continue;
            }
        } else {
            drop_cols.clear();
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// measure.json do jeito que o agente precisa: compacto e sem os percentis de luma (so o autograde usa,
/// e ele le o arquivo completo). Era ~39 KB indentado relido em todo turno.
fn slim_measure(measure: &Value, dst: &Path) -> Result<()> {
    let mut m = measure.clone();
    if let Some(shots) = m["shots"].as_array_mut() {
        for s in shots {
            let p50 = s["luma"]["p50"].clone();
            s["luma"] = p50;
        }
    }
    std::fs::write(dst, serde_json::to_string(&m)?)?;
    Ok(())
}

/// Copia o report.md sem as secoes de `REPORT_SKIP` para report_agent.md e devolve o caminho.
fn slim_report(adir: &Path) -> Result<PathBuf> {
    let src = adir.join("report.md");
    let text = read(&src);
    if text.is_empty() {
        return Ok(src);
    }
    let out = drop_inferences(&agent::drop_sections(&text, &REPORT_SKIP));
    let dst = adir.join("report_agent.md");
    std::fs::write(&dst, out)?;
    Ok(dst)
}

pub async fn recreate(ch: &Channel, o: Opts) -> Result<PathBuf> {
    let span = crate::events::step(&ch.slug, "recreate");
    span.action("Recriando o edit", "claude");
    let r = recreate_inner(ch, o).await;
    span.finish(r, |p| p.display().to_string())
}

pub fn character_clips(ch: &Channel, name: &str) -> usize {
    let known = footage::known_names(ch);
    footage::load_bank(ch).iter().filter(|c| c.usable && footage::is_character(c, Some(name), &known)).count()
}

async fn recreate_inner(ch: &Channel, o: Opts) -> Result<PathBuf> {
    // personagem sem clipe nenhum: antes so avisava, analisava a ref, mandava "(banco vazio)" para o agente e
    // gastava tokens num plano impossivel (ou montado com outro personagem). Confere antes de qualquer trabalho.
    if let Some(p) = &o.character {
        let n = character_clips(ch, p);
        anyhow::ensure!(
            n > 0,
            "nenhum clipe de imagem de `{p}` no banco do {}: rode `tkd footage --channel {} --character \"{p}\"` antes (e --speech en para falas)",
            ch.slug,
            ch.slug
        );
        if n < 8 {
            tracing::warn!(
                "[recreate] so {n} clipes de imagem de `{p}` no banco: rode `tkd footage --channel {} --character \"{p}\"` (e --speech en para falas)",
                ch.slug
            );
        }
    }
    let (ref_mp4, ref_id) = resolve_ref(ch, &o.ref_)?;
    let adir = analyze_ref(ch, &ref_mp4, &ref_id).await?;
    let measure_p = adir.join("measure.json");
    let measure: Value = serde_json::from_str(&read(&measure_p)).context("measure.json")?;
    let fps = measure["probe"]["fps"].as_f64().unwrap_or(30.0).round();
    let dur = measure["probe"]["duration"].as_f64().unwrap_or(10.0);
    let end_s = o.end_s.unwrap_or(dur).min(dur);
    let total = (end_s * fps).round() as i64;
    // formato do CANAL, nao o da ref: copiamos ritmo/estrutura, e o banco ja foi recortado no formato do canal.
    // Em 17/11 a ref do canal de edits era quadrada (1080x1078) e o edit saiu quadrado.
    let (aw, ah) = footage::channel_aspect(ch);
    let (w, h) = (1080i64, 1080 * ah as i64 / aw as i64);
    let (rw, rh) = (measure["probe"]["width"].as_i64().unwrap_or(0), measure["probe"]["height"].as_i64().unwrap_or(0));
    if rw > 0 && rh > 0 && (rh * w - h * rw).abs() > rw * h / 50 {
        tracing::info!("[recreate] ref {rw}x{rh}, edit sai {w}x{h} (formato do canal)");
    }

    // trilha: a musica da propria ref (stems) ou a previa do Shazam; nenhuma das duas gasta YouTube
    let ref_track = music::ensure_ref_track(ch, &ref_id, &ref_mp4, &adir).await;

    let run_id = format!("{}-{}", ref_id, Local::now().format("%Y%m%d-%H%M%S"));
    let rdir = root().join("analysis").join(&ch.slug).join("recreate").join(&run_id);
    std::fs::create_dir_all(&rdir)?;

    // 2. plano: do agente ou de um arquivo pronto
    let agent_out: Value = if let Some(p) = &o.plan {
        let v: Value = serde_json::from_str(&read(p)).with_context(|| format!("lendo {}", p.display()))?;
        if v.get("plan").is_some() { v } else { json!({ "plan": v }) }
    } else {
        let script = if o.script.trim().is_empty() {
            "(sem roteiro fixo) Escolha FALAS REAIS do banco que, em sequencia, formem um monologo coerente e forte \
             no tom do canal; a ultima fala/palavra de impacto vira o titulo."
                .to_string()
        } else {
            o.script.clone()
        };
        let min_distinct = o.character.as_deref().map(|p| character_clips(ch, p).min(MIN_DISTINCT_CLIPS)).unwrap_or(0);
        let example = read(&root().join("agents/recreate_example.json"));
        let handle = if ch.post_handle.is_empty() { ch.slug.clone() } else { ch.post_handle.clone() };
        let rea_report = slim_report(&adir)?;
        let measure_agent = adir.join("measure_agent.json");
        slim_measure(&measure, &measure_agent)?;
        let vars: HashMap<&str, String> = HashMap::from([
            ("slug", ch.slug.clone()),
            ("ref_mp4", rel(&ref_mp4)),
            ("end_s", format!("{end_s:.2}")),
            ("end_frames", total.to_string()),
            ("fps", (fps as i64).to_string()),
            ("width", w.to_string()),
            ("height", h.to_string()),
            ("measure", rel(&measure_agent)),
            ("rea_report", rel(&rea_report)),
            ("sheet", rel(&sheet_path(&adir))),
            ("guides", guides(ch)),
            ("script", script.clone()),
            ("clips", footage::bank_digest_for(ch, o.character.as_deref())),
            ("character", o.character.clone().unwrap_or_else(|| "(nao definido: escolha UM personagem central pela ref e use so ele)".into())),
            ("tracks", music::digest_for(ch, Some(&ref_id))),
            ("example", example),
            ("watermark", format!("© {} - ALL RIGHTS RESERVED", handle.to_uppercase())),
            ("voice_id", o.voice_id.clone()),
            ("handle", handle.clone()),
            ("speech", footage::speech_digest(ch, o.character.as_deref())),
            ("ref_speech", {
                let w = ref_speech_windows(&measure, end_s);
                if w.is_empty() { "(a ref nao tem fala)".into() } else { fmt_windows(&w) }
            }),
            ("quality", read(&root().join("docs").join("QUALITY_RULES.md"))),
            ("lang", o.lang.clone()),
            ("voice_mode", o.voice.clone()),
            ("min_clips", min_distinct.max(1).to_string()),
        ]);
        let prompt = agent::render_template("recreate.md", &vars)?;
        let bank = footage::load_bank(ch);
        let ref_speech = ref_speech_windows(&measure, end_s);
        let (v, left) = agent::ask_json_checked(ch, "recreate", &prompt, &["Read", "Glob"], 1500, 2, 2, |v: &Value| {
            check_plan(v, &bank, o.character.as_deref(), fps, &ref_speech, min_distinct)
        })
        .await?;
        for e in &left {
            tracing::warn!("[recreate] erro que o agente nao corrigiu: {e}");
        }
        v
    };
    std::fs::write(rdir.join("agent.json"), serde_json::to_string_pretty(&agent_out)?)?;
    if let Some(n) = agent_out["notes"].as_str() {
        tracing::info!("[recreate] notas do agente: {n}");
    }

    // 3. validacao
    let mut plan = agent_out["plan"].clone();
    let n = crate::brand::scrub_json(ch, &mut plan);
    if n > 0 {
        tracing::warn!("[recreate] plano citava as referencias {n}x: trocado pela marca do canal");
    }
    plan["fps"] = json!(fps);
    plan["width"] = json!(w);
    plan["height"] = json!(h);
    plan["durationInFrames"] = json!(total);
    plan["id"] = json!(run_id);
    if let Some(t) = &ref_track {
        if fill_music(&mut plan, &t.file) {
            tracing::info!("[recreate] plano sem trilha: usando a musica da propria ref ({})", t.file);
        }
    }
    for wmsg in validate(&mut plan, ch, total)? {
        tracing::warn!("[recreate] {wmsg}");
    }
    let plan_p = rdir.join("plan.json");
    std::fs::write(&plan_p, serde_json::to_string_pretty(&plan)?)?;

    // 4. cor por plano
    python(
        "autograde",
        &["scripts/autograde.py", &rel(&plan_p), &rel(&measure_p), &rel(&footage::public_dir())],
    )
    .await?;

    // 4b. movimento: punch/shake/rgb/flash/pulse a partir dos cortes e dos onsets reais da trilha
    {
        let mut args = vec!["scripts/automotion.py".to_string(), rel(&plan_p), rel(&footage::public_dir())];
        if let Some(m) = &o.motion {
            args.extend(["--preset".into(), m.clone()]);
        }
        let r: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        python("automotion", &r).await?;
    }

    // 5. voz: falas reais dos clipes (padrao quando o plano usa) ou TTS
    let mut asm: Vec<String> = vec!["scripts/assemble_plan.py".into(), rel(&plan_p), rel(&rdir.join("props.json"))];
    let lines = agent_out["voice"]["lines"].as_array().cloned().unwrap_or_default();
    let real_speech = plan["shots"].as_array().map(|v| v.iter().any(|s| s.get("speech").is_some())).unwrap_or(false)
        || plan["voiceClips"].as_array().map(|v| !v.is_empty()).unwrap_or(false);
    if o.voice != "none" && o.voice != "tts" && real_speech {
        let vj = rdir.join("voice.json");
        python(
            "clip_voice (falas reais)",
            &["scripts/clip_voice.py", &rel(&plan_p), &rel(&footage::manifest_path(ch)), &rel(&vj), "--lang", &o.lang],
        )
        .await?;
        asm.extend(["--voice".into(), rel(&vj)]);
    } else if o.voice == "clip" {
        tracing::warn!("[recreate] --voice clip, mas o plano nao usou nenhuma fala do banco (rode `tkd footage --speech`)");
    } else if o.voice != "none" && !lines.is_empty() {
        if std::env::var("ELEVENLABS_API_KEY").map(|k| k.is_empty()).unwrap_or(true) {
            tracing::warn!("[recreate] ELEVENLABS_API_KEY ausente: video sai sem voz e sem legenda");
        } else {
            let pub_rel = format!("{}/recreate/{}", ch.slug, run_id);
            let out_dir = footage::public_dir().join(&pub_rel);
            let mut vp = agent_out["voice"].clone();
            if vp["voice_id"].as_str().unwrap_or("").is_empty() {
                vp["voice_id"] = json!(o.voice_id);
            }
            if vp.get("model_id").is_none() {
                vp["model_id"] = json!("eleven_multilingual_v2");
            }
            if vp.get("voice_settings").is_none() {
                vp["voice_settings"] = json!({"stability": 0.6, "similarity_boost": 0.8, "style": 0.2, "use_speaker_boost": true});
            }
            vp["fps"] = json!(fps);
            vp["duration"] = json!(end_s);
            vp["out_dir"] = json!(rel(&out_dir));
            let vp_p = rdir.join("voice_plan.json");
            std::fs::write(&vp_p, serde_json::to_string_pretty(&vp)?)?;
            python("voice_lines (ElevenLabs)", &["scripts/voice_lines.py", &rel(&vp_p)]).await?;
            asm.extend([
                "--voice".into(),
                rel(&out_dir.join("voice.json")),
                "--voice-wav".into(),
                format!("{pub_rel}/voice.wav"),
            ]);
        }
    }
    // 6. props
    let asm_ref: Vec<&str> = asm.iter().map(|s| s.as_str()).collect();
    python("assemble_plan", &asm_ref).await?;
    {
        let props: Value = serde_json::from_str(&std::fs::read_to_string(rdir.join("props.json"))?)?;
        if let Some(why) = silent_reason(&props) {
            bail!("video sairia mudo ({why}); nada renderizado");
        }
    }

    // 7. render
    let out = root().join("out").join("recreate").join(format!("{run_id}.mp4"));
    std::fs::create_dir_all(out.parent().unwrap())?;
    let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
    let st = Command::new(npx)
        .args([
            "remotion",
            "render",
            "recreate",
            &out.to_string_lossy(),
            &format!("--props={}", rdir.join("props.json").to_string_lossy()),
            "--log=error",
            // so velocidade (mesmos quadros): 2 abas do Chromium com voce no PC, 4 quando esta fora
            if crate::idle::is_away() { "--concurrency=4" } else { "--concurrency=2" },
            // qualidade alta para todo render (regra global: docs/QUALITY_RULES.md)
            "--crf=16",
            "--x264-preset=slow",
            "--jpeg-quality=95",
            "--audio-bitrate=320k",
        ])
        .current_dir(root().join("video"))
        .tree_status()
        .await
        .context("npx remotion render")?;
    anyhow::ensure!(st.success() && out.exists(), "remotion render falhou ({st})");
    // loudness final no alvo das refs (edits de anime ~ -12 LUFS); fala real dos clipes vem bem mais baixa que TTS normalizado
    let lufs = std::env::var("TKD_LUFS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(-12.0);
    if !loudnorm(&out, lufs).await? {
        tracing::warn!("[recreate] loudnorm falhou; mp4 fica com o volume do render");
    }
    tracing::info!("[recreate] mp4: {}", out.display());

    // 7b. portao de qualidade (docs/QUALITY_RULES.md): erro = nao segue para legenda/avaliador/review
    // (antes so registrava o qa.json e seguia gastando com um video que a regra manda nao publicar)
    {
        let qa = rdir.join("qa.json");
        let mut args = vec![
            "scripts/qa.py".to_string(), rel(&out), rel(&rdir.join("props.json")),
            rel(&footage::manifest_path(ch)), rel(&qa), "--lufs".into(), format!("{lufs}"),
        ];
        if let Some(c) = &o.character {
            args.extend(["--character".into(), c.clone()]);
        }
        let r: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let run_qa = || async {
            let _ = std::fs::remove_file(&qa);
            match python("qa", &r).await {
                Ok(()) => serde_json::from_str::<Value>(&read(&qa)).ok(),
                Err(e) => {
                    tracing::warn!("[recreate] QA nao rodou: {e}");
                    None
                }
            }
        };
        if let Some(mut res) = run_qa().await {
            let mut errs = qa_errors(&res);
            // loudness e o unico erro que se conserta sem renderizar: mais uma passada e confere de novo
            if only_loudness(&errs) {
                tracing::warn!("[recreate] QA: {}; nova passada do loudnorm", errs.join("; "));
                if loudnorm(&out, lufs).await.unwrap_or(false) {
                    if let Some(r2) = run_qa().await {
                        res = r2;
                        errs = qa_errors(&res);
                    }
                }
            }
            if !errs.is_empty() {
                bail!("QA reprovou o edit ({}); nada de legenda/avaliação. Detalhes: {}", errs.join("; "), rel(&qa));
            }
        }
    }

    // 8. validacao visual
    if o.compare {
        let mut c = Command::new(bash());
        c.args(["scripts/compare.sh", &rel(&ref_mp4), &rel(&out), &rel(&rdir.join("compare")), &format!("{end_s:.3}")]);
        if let Err(e) = run("compare.sh", &mut c).await {
            tracing::warn!("[recreate] comparacao falhou: {e}");
        }
    }
    if !out.exists() {
        bail!("sem mp4");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn speaks_where_ref_speaks() {
        let m = json!({"voice": [{"start": 0.2, "end": 2.58}, {"start": 3.01, "end": 4.43}, {"start": 4.43, "end": 6.54},
                                 {"start": 6.54, "end": 13.85}, {"start": 13.85, "end": 16.41}, {"start": 16.84, "end": 19.84}]});
        let w = ref_speech_windows(&m, 27.7);
        assert_eq!(w, vec![(0.2, 6.54), (13.85, 19.84)]);
        // plano com fala so no 1o trecho: erro no 2o
        let plan = json!({"plan": {"shots": [], "voiceClips": []}});
        let e = check_plan(&plan, &[], None, 30.0, &w, 0);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[1].contains("13.8-19.8s"));
    }

    #[test]
    fn plan_needs_variety() {
        let shots = |srcs: &[(&str, usize)]| -> Vec<Value> {
            srcs.iter().flat_map(|(s, n)| std::iter::repeat_n(json!({"src": s}), *n)).collect()
        };
        // o edit do Naoya de 18/11: 87 planos de 4 clipes, um repetido 38 vezes
        let naoya = shots(&[("a", 38), ("b", 23), ("c", 16), ("d", 10)]);
        let e = check_variety(&naoya, 8);
        assert!(e[0].contains("so 4 clipes"), "{e:?}");
        assert!(e.iter().any(|x| x.contains("a aparece em 38")) && e.len() == 3, "{e:?}");
        // banco pequeno: exige so o que existe; espalhado sem clipe dominante passa
        let ok = shots(&[("a", 10), ("b", 10), ("c", 10), ("d", 10), ("e", 10)]);
        assert!(check_variety(&ok, 5).is_empty());
        assert!(check_variety(&naoya, 0).is_empty());
        // poucos planos: nao cobra mais clipes do que planos
        assert!(check_variety(&shots(&[("a", 1), ("b", 1), ("c", 1), ("d", 1)]), 8).is_empty());
    }

    #[test]
    fn plan_checked_against_catalog() {
        use crate::footage::{Clip, FrameTag};
        let mk = |file: &str, dur: f64, frames: Vec<(&[&str], &[&str])>, speech: serde_json::Value| {
            let mut c: Clip = serde_json::from_value(json!({"file": file, "source": "", "source_title": "",
                "start": 0, "duration": dur, "motion": 0, "speech": speech})).unwrap();
            c.frames = frames.into_iter().map(|(ch, fa)| FrameTag { t: 0.0, a: 0.0, b: 0.0,
                chars: ch.iter().map(|s| s.to_string()).collect(), faces: fa.iter().map(|s| s.to_string()).collect() }).collect();
            c
        };
        let speech_line = json!([{"start": 0.2, "end": 1.9, "text": "count me in on all the fun", "lang": "en",
            "words": [{"w":"count","s":0.2,"e":0.4},{"w":"me","s":0.4,"e":0.5},{"w":"in","s":0.5,"e":0.6},{"w":"on","s":0.6,"e":0.7},
                      {"w":"all","s":0.7,"e":0.9},{"w":"the","s":0.9,"e":1.0},{"w":"fun","s":1.0,"e":1.9}]}]);
        let bank = vec![
            mk("gojo.mp4", 3.0, vec![(&["Satoru Gojo"], &[]), (&[], &[]), (&[], &[])], json!([])),
            mk("toji_face.mp4", 2.0, vec![(&["Toji Fushiguro"], &["Toji Fushiguro"])], json!([])),
            mk("costas.mp4", 2.0, vec![(&["desconhecido"], &[])], json!([])),
            mk("fala.mp4", 2.0, vec![(&[], &[])], speech_line),
        ];
        let plan = |shots: serde_json::Value, title: serde_json::Value| json!({"plan": {"shots": shots,
            "voiceClips": [{"src": "fala.mp4", "line": 0, "at": 0, "captions": [{"kind": "title", "words": title}]}]}});
        // Gojo no trecho 0-0.5 s, rosto do Toji sob a fala so-audio, palavra inexistente
        let bad = plan(json!([{"src": "gojo.mp4", "from": 0, "to": 15, "startAt": 0.0},
                              {"src": "toji_face.mp4", "from": 15, "to": 60, "startAt": 0.0}]), json!(["funn"]));
        let e = check_plan(&bad, &bank, Some("Toji Fushiguro"), 30.0, &[], 0);
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].contains("Satoru Gojo") && e.iter().any(|x| x.contains("rosto de Toji")) && e.iter().any(|x| x.contains("funn")));
        // corrigido: trecho do clipe sem o Gojo, costas sob a fala, titulo pela palavra certa
        let good = plan(json!([{"src": "gojo.mp4", "from": 0, "to": 15, "startAt": 1.1},
                               {"src": "costas.mp4", "from": 15, "to": 60, "startAt": 0.0}]), json!(["fun"]));
        assert!(check_plan(&good, &bank, Some("Toji Fushiguro"), 30.0, &[], 0).is_empty());
        // clipe inventado e fala que o clipe nao tem: erros que o agente recebe para corrigir (G2 do CHAOS.md)
        let ghost = json!({"plan": {"shots": [{"src": "nao_existe.mp4", "from": 0, "to": 60, "startAt": 0.0}],
            "voiceClips": [{"src": "fala.mp4", "line": 7, "at": 0}, {"src": "sumiu.mp4", "line": 0, "at": 0}]}});
        let e = check_plan(&ghost, &bank, Some("Toji Fushiguro"), 30.0, &[], 0);
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].contains("nao_existe.mp4") && e[1].contains("nao tem fala 7") && e[2].contains("sumiu.mp4"));
    }
    use super::*;

    #[test]
    fn report_without_guesses() {
        let md = "## Shot list\n\n| # | Time | Camera | Made with (inferred) | Material (inferred) | Remake |\n|---|---|---|---|---|---|\n| 1 | 0:00.0 | static | AI 44% | AI-generated | AI · G1 |\n\nText after.\n\n<details><summary>#1</summary>\n\n**Visible on screen** (check)\n- Camera / lighting: close-up\n\n**How it was made, as inferred by the tools** (inferred)\n- Inference: AI 44%\n\n- Remake: Source is AI-generated\n\n</details>\n";
        let out = drop_inferences(md);
        assert!(out.contains("| # | Time | Camera |\n"));
        assert!(out.contains("| 1 | 0:00.0 | static |\n"));
        assert!(out.contains("Text after."));
        assert!(out.contains("- Camera / lighting: close-up"));
        assert!(out.contains("</details>"));
        assert!(!out.contains("Inference") && !out.contains("Remake") && !out.contains("Made with"));
    }

    /// 1o `analysis/<canal>/<video>/report.md` que tiver `measure.json` ao lado.
    fn first_report() -> Option<PathBuf> {
        for ch in std::fs::read_dir(root().join("analysis")).ok()?.flatten() {
            for v in std::fs::read_dir(ch.path()).into_iter().flatten().flatten() {
                let r = v.path().join("report.md");
                if r.exists() && v.path().join("measure.json").exists() {
                    return Some(r);
                }
            }
        }
        None
    }

    /// Medida num relatorio real de `analysis/` (pula se nao houver analise na maquina).
    #[test]
    fn real_report_shrinks() {
        let Some(p) = first_report() else { return };
        let Ok(text) = std::fs::read_to_string(&p) else { return };
        let prior = agent::drop_sections(&text, &REPORT_SKIP[..5]).len();
        let later = drop_inferences(&agent::drop_sections(&text, &REPORT_SKIP)).len();
        eprintln!("report_agent: {prior} -> {later} bytes");
        assert!(later * 2 < prior);
        let m: Value = serde_json::from_str(&std::fs::read_to_string(p.with_file_name("measure.json")).unwrap()).unwrap();
        let dst = std::env::temp_dir().join("tkd-measure-agent-test.json");
        slim_measure(&m, &dst).unwrap();
        let n = std::fs::metadata(&dst).unwrap().len();
        let full = std::fs::metadata(p.with_file_name("measure.json")).unwrap().len();
        eprintln!("measure: {full} -> {n} bytes");
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn muted_video_is_blocked() {
        // caso do canal de edits em 17/11: sem trilha e voiceClips sem from/to
        let muted = json!({"audio": {}, "shots": [{"volume": 0}], "voiceClips": [{"src": "a.mp4", "line": 0, "at": 261}]});
        assert!(silent_reason(&muted).unwrap().contains("sem from/to"));
        assert!(silent_reason(&json!({"audio": {}, "shots": []})).unwrap().contains("sem trilha"));
        assert!(silent_reason(&json!({"audio": {"music": "m.mp3"}, "shots": []})).is_none());
        let speech_line = json!({"audio": {}, "shots": [], "voiceClips": [{"src": "a.mp4", "from": 0, "to": 60}]});
        assert!(silent_reason(&speech_line).is_none());
        assert!(silent_reason(&json!({"audio": {}, "shots": [{"volume": 1}]})).is_none());
    }

    #[test]
    fn plan_without_music_gets_ref_music() {
        let mut p = json!({"shots": []});
        assert!(fill_music(&mut p, "canal/audio/ref-1.mp3"));
        assert_eq!(p["audio"]["music"], "canal/audio/ref-1.mp3");
        assert_eq!(p["audio"]["musicStart"], 0);
        // escolha do agente e mantida
        let mut q = json!({"audio": {"music": "outra.mp3", "musicStart": 3.5}});
        assert!(!fill_music(&mut q, "canal/audio/ref-1.mp3"));
        assert_eq!(q["audio"]["music"], "outra.mp3");
    }

    #[test]
    fn qa_error_list() {
        let qa = json!({"issues": [
            {"level": "erro", "what": "loudness -16.0 LUFS fora do alvo -12±1.5", "where": ""},
            {"level": "aviso", "what": "bitrate baixo", "where": ""}]});
        let e = qa_errors(&qa);
        assert_eq!(e.len(), 1);
        assert!(only_loudness(&e));
        let qa2 = json!({"issues": [{"level": "erro", "what": "video sem audio", "where": ""}]});
        assert!(!only_loudness(&qa_errors(&qa2)));
        assert!(qa_errors(&json!({"issues": []})).is_empty());
    }

    #[test]
    fn report_slim() {
        let d = std::env::temp_dir().join(format!("tkd-slim-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("report.md"),
            "# T\n## Shot list\nA\n## Remake\n### AI generation\nX\n#### sub\nX\n### Edit\nB\n## Open questions\nX\n## Sound\nC\n",
        )
        .unwrap();
        let out = std::fs::read_to_string(slim_report(&d).unwrap()).unwrap();
        assert_eq!(out, "# T\n## Shot list\nA\n## Remake\n### Edit\nB\n## Sound\nC\n");
        std::fs::remove_dir_all(&d).ok();
    }
}
