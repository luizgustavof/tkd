use crate::proc::TreeExt;
use crate::{agent, config::Channel, media, state::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct Queries {
    queries: Vec<Query>,
    /// nomes proprios falados nas cenas (vao como hotwords pro whisper)
    #[serde(default)]
    names: Vec<String>,
}

fn names_path(ch: &Channel) -> PathBuf {
    ch.knowledge_dir().join("names.json")
}

/// Nomes proprios do canal (acumulados das buscas) + o personagem: vocabulario do whisper.
fn hotwords(ch: &Channel, new_items: &[String], character: Option<&str>) -> String {
    let mut v: Vec<String> = crate::config::load_json(&names_path(ch));
    for n in new_items.iter().map(|n| n.trim()).filter(|n| !n.is_empty() && n.len() <= 40) {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            v.push(n.to_string());
        }
    }
    let _ = crate::config::save_json(&names_path(ch), &v);
    let mut all = v;
    if let Some(c) = character {
        if !all.iter().any(|x| x.eq_ignore_ascii_case(c)) {
            all.insert(0, c.to_string());
        }
    }
    all.join(", ")
}

#[derive(Debug, Deserialize)]
struct Query {
    q: String,
    #[serde(default)]
    why: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "ten")]
    max_minutes: u64,
}

fn ten() -> u64 {
    10
}

/// Um plano recortado de um video da internet, pronto para entrar numa cena.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    /// caminho relativo a video/public/ — e o que vai na spec
    pub file: String,
    pub source: String,
    pub source_title: String,
    pub start: f64,
    pub duration: f64,
    pub motion: f64,
    #[serde(default)]
    pub palette: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub mood: String,
    #[serde(default)]
    pub best_for: String,
    #[serde(default)]
    pub quality: f64,
    #[serde(default = "yes_default")]
    pub usable: bool,
    /// falas reais do clipe (scripts/speech.py): [{start,end,text,lang,prob,words:[{w,s,e}]}], tempos no clipe
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speech: Vec<serde_json::Value>,
    /// altura do video de origem (o recorte e sempre 1080; < 720 = ampliado, imagem mole)
    #[serde(default)]
    pub src_height: u32,
    /// blocagem de compressao (scripts/clip_quality.py; > 1.5 = macrobloco visivel)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<f64>,
    /// nota do momento da fonte onde o recorte caiu (scripts/moments.py; 0 = sem detector)
    #[serde(default)]
    pub moment: f64,
    /// quem aparece em cada frame do catalogo (frames em duracao*(i+0.5)/n): base do filtro de personagem unico
    /// e da regra "fala so-audio nunca sobre rosto". Vazio = clipe catalogado antes de 18/11.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<FrameTag>,
}

/// Um frame catalogado: personagens visiveis e de quem o rosto/boca aparece.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrameTag {
    /// segundos no clipe
    pub t: f64,
    /// cena do clipe que este frame representa [a, b] (s); b = 0 = fatia igual (catalogo antigo)
    #[serde(default)]
    pub a: f64,
    #[serde(default)]
    pub b: f64,
    #[serde(default)]
    pub chars: Vec<String>,
    #[serde(default)]
    pub faces: Vec<String>,
}

/// Figura que o catalogo nao identificou (costas, silhueta, longe): neutra, nao proibe nem prova personagem.
pub fn unknown_person(tag: &str) -> bool {
    let t = tag.trim().to_lowercase();
    t.is_empty() || t.starts_with("desconhecid") || t.starts_with("unknown") || t.starts_with("figura")
}

/// Palavras que nao identificam ninguem (artigo, particula de nome, honorifico, palavra comum).
const NAME_STOP: &[&str] = &[
    "que", "nao", "não", "the", "and", "von", "van", "der", "den", "del", "dos", "das", "com", "sem", "for", "sama",
    "san", "kun", "chan",
];

/// Partes de um nome que identificam: minusculas, 3+ letras, sem `NAME_STOP`.
pub fn name_tokens(x: &str) -> Vec<String> {
    x.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| p.chars().count() >= 3 && !NAME_STOP.contains(p))
        .map(String::from)
        .collect()
}

/// Mesmo personagem? As partes do nome mais curto tem que estar todas no outro: "Toji" e "Fushiguro Toji" casam
/// com "Toji Fushiguro"; "Megumi Fushiguro" nao (antes bastava uma parte: o sobrenome juntava pai e filho, e
/// "Monkey D. Luffy" casava com qualquer "Monkey").
pub fn same_person(tag: &str, name: &str) -> bool {
    let (a, b) = (name_tokens(tag), name_tokens(name));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    short.iter().all(|p| long.contains(p))
}

/// Personagens que o canal conhece (channel.toml + nomes acumulados nas buscas): dizem quais partes de um nome
/// sao ambiguas (sobrenome de familia, titulo).
pub fn known_names(ch: &Channel) -> Vec<String> {
    let mut v = ch.characters.clone();
    for n in crate::config::load_json::<Vec<String>>(&names_path(ch)) {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            v.push(n);
        }
    }
    v
}

impl Clip {
    /// Trecho [a, b] em segundos de cada frame catalogado (o frame i representa a i-esima fatia do clipe).
    pub fn frame_spans(&self) -> Vec<(f64, f64, &FrameTag)> {
        let n = self.frames.len().max(1) as f64;
        self.frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if f.b > f.a {
                    (f.a, f.b, f)
                } else {
                    (self.duration * i as f64 / n, self.duration * (i + 1) as f64 / n, f)
                }
            })
            .collect()
    }

    /// Outros personagens visiveis entre `a` e `b` segundos (edit de personagem unico).
    pub fn others_in(&self, a: f64, b: f64, character: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for (s, e, f) in self.frame_spans() {
            if s < b - 0.02 && e > a + 0.02 {
                for c in &f.chars {
                    if !unknown_person(c) && !same_person(c, character) && !out.contains(c) {
                        out.push(c.clone());
                    }
                }
            }
        }
        out
    }

    /// Algum rosto/boca visivel entre `a` e `b` segundos.
    pub fn face_in(&self, a: f64, b: f64) -> Option<String> {
        self.frame_spans()
            .into_iter()
            .filter(|(s, e, _)| *s < b - 0.02 && *e > a + 0.02)
            .find_map(|(_, _, f)| f.faces.first().cloned())
    }
}

/// Saida do scripts/moments.py.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MomentSet {
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub weights: serde_json::Value,
    #[serde(default)]
    pub signals: serde_json::Value,
    /// nota combinada por segundo
    #[serde(default)]
    pub curve: Vec<f64>,
    #[serde(default)]
    pub moments: Vec<MomentPick>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MomentPick {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub peak: i64,
    pub score: f64,
    #[serde(default)]
    pub breakdown: serde_json::Value,
}

impl MomentSet {
    /// media da curva no intervalo [a, b) em segundos.
    pub fn rating(&self, a: f64, b: f64) -> f64 {
        let n = self.curve.len();
        if n == 0 {
            return 0.0;
        }
        let i = (a.max(0.0) as usize).min(n - 1);
        let j = ((b.ceil() as usize).max(i + 1)).min(n);
        self.curve[i..j].iter().sum::<f64>() / (j - i) as f64
    }
}

/// Como chamar o detector para uma fonte: meta do YouTube + perfil/lexico do canal.
#[derive(Debug, Clone, Default)]
pub struct MomCtx {
    pub meta: Option<PathBuf>,
    pub profile: String,
    pub lexicon: Vec<String>,
    pub weights: std::collections::BTreeMap<String, f64>,
}

impl MomCtx {
    pub fn for_channel(ch: &Channel, speech_lines: bool, character: Option<&str>) -> Self {
        let m = &ch.moments;
        let mut lexicon = m.lexicon.clone();
        if let Some(p) = character {
            lexicon.extend(p.split_whitespace().filter(|w| w.len() > 2).map(String::from));
            lexicon.push(p.to_string());
        }
        let profile = if !m.profile.is_empty() {
            m.profile.clone()
        } else if speech_lines {
            "speech".into()
        } else {
            "fight".into()
        };
        Self { meta: None, profile, lexicon, weights: m.weights.clone() }
    }
}

/// Roda scripts/moments.py. Falha = None (o chamador segue com o criterio antigo).
pub async fn find_moments(source_path: &Path, ctx: &MomCtx, speech_json: Option<&Path>, top: usize) -> Option<MomentSet> {
    let out = std::env::temp_dir().join(format!(
        "tkd-moments-{}.json",
        source_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let mut args: Vec<String> = vec![
        "-I".into(), "-X".into(), "utf8".into(), "scripts/moments.py".into(),
        source_path.to_string_lossy().to_string(), out.to_string_lossy().to_string(),
        "--profile".into(), ctx.profile.clone(), "--top".into(), top.to_string(),
    ];
    if let Some(m) = ctx.meta.as_ref().filter(|m| m.exists()) {
        args.extend(["--meta".into(), m.to_string_lossy().to_string()]);
    }
    if let Some(s) = speech_json {
        args.extend(["--speech".into(), s.to_string_lossy().to_string()]);
    }
    if !ctx.lexicon.is_empty() {
        args.extend(["--lexicon".into(), ctx.lexicon.join("|")]);
    }
    if !ctx.weights.is_empty() {
        args.extend(["--weights".into(), serde_json::to_string(&ctx.weights).unwrap_or_default()]);
    }
    let st = Command::new(crate::recreate::venv_bin("python"))
        .args(&args)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("TKD_FFMPEG", media::ffmpeg_bin())
        .current_dir(crate::config::root())
        .tree_output()
        .await;
    let ok = match st {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            tracing::warn!("moments.py falhou: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
            false
        }
        Err(e) => {
            tracing::warn!("moments.py nao rodou: {e}");
            false
        }
    };
    let r = if ok {
        std::fs::read_to_string(&out).ok().and_then(|t| serde_json::from_str::<MomentSet>(&t).ok())
    } else {
        None
    };
    let _ = std::fs::remove_file(&out);
    if let Some(m) = &r {
        tracing::info!(
            "momentos: {} picos, pesos {}, sinais {}",
            m.moments.len(), m.weights, m.signals
        );
    }
    r
}

/// `crop=...,` que vem antes do recorte no formato do canal (ou vazio): tira a tarja preta da fonte e, se a fonte
/// tem legenda queimada, corta tudo da linha `subs_top` (fracao da altura) para baixo.
async fn bars_filter(source_path: &Path, duration: f64, subs_top: Option<f64>) -> String {
    let bars = media::detect_bars(source_path, duration).await;
    if let Some(c) = &bars {
        tracing::info!("fonte com tarja preta: {c} antes do recorte");
    }
    let Some(top) = subs_top else {
        return bars.map(|c| format!("{c},")).unwrap_or_default();
    };
    let Ok(p) = media::probe(source_path).await else { return bars.map(|c| format!("{c},")).unwrap_or_default() };
    let (w, h, x, y) = bars
        .as_deref()
        .and_then(|c| media::parse_crop(c.trim_start_matches("crop=")))
        .unwrap_or((p.width as i64, p.height as i64, 0, 0));
    let (crop_h, crop_y) = subs_crop(h, y, top, p.height as i64);
    tracing::info!("fonte com legenda queimada a partir de {:.0}% da altura: recorte {w}x{crop_h}", top * 100.0);
    format!("crop={w}:{crop_h}:{x}:{crop_y},")
}

/// Altura e y do recorte que para 1,5% acima da legenda (dentro da area sem tarja `h` a partir de `y`).
fn subs_crop(h: i64, y: i64, top: f64, full_h: i64) -> (i64, i64) {
    let limit = (top - 0.015) * full_h as f64;
    let crop_h = ((limit as i64 - y).min(h) / 2) * 2;
    (crop_h.max(2), y)
}

/// Resultado do detector de legenda queimada na fonte baixada.
enum SubsCheck {
    Clean,
    /// legenda a partir desta fracao da altura: recortar acima
    Crop(f64),
    Reject(String),
}

/// OCR na faixa de baixo da fonte (`scripts/burned_subs.py`). Recorte acima da legenda so se sobrarem >= 720 px
/// de altura (regra de qualidade); senao a fonte e descartada. Detector que falha nao bloqueia (so avisa).
async fn check_burned_subs(source_path: &Path) -> SubsCheck {
    let o = Command::new(crate::recreate::venv_bin("python"))
        .args(["-I", "-X", "utf8", "scripts/burned_subs.py", "source", &source_path.to_string_lossy()])
        .env("PYTHONUTF8", "1")
        .current_dir(crate::config::root())
        .tree_output()
        .await;
    let v: Option<serde_json::Value> = o
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().last().and_then(|l| serde_json::from_str(l).ok()));
    let Some(v) = v else {
        tracing::warn!("detector de legenda queimada nao rodou; seguindo sem checar");
        return SubsCheck::Clean;
    };
    if !v["has_subs"].as_bool().unwrap_or(false) {
        return SubsCheck::Clean;
    }
    let top = v["top"].as_f64().unwrap_or(0.0);
    let height = media::probe(source_path).await.map(|p| p.height).unwrap_or(0) as f64;
    let sample_text = v["texts"][0].as_str().unwrap_or("").to_string();
    if top >= 0.7 && (top - 0.015) * height >= 720.0 {
        SubsCheck::Crop(top)
    } else {
        SubsCheck::Reject(format!("legenda queimada (\"{sample_text}\") alta demais para recortar"))
    }
}

fn yes_default() -> bool {
    true
}

pub fn public_dir() -> PathBuf {
    crate::config::root().join("video").join("public")
}

pub fn clips_dir(ch: &Channel) -> PathBuf {
    public_dir().join(&ch.slug).join("clips")
}

pub fn manifest_path(ch: &Channel) -> PathBuf {
    ch.knowledge_dir().join("clips.json")
}

pub fn load_bank(ch: &Channel) -> Vec<Clip> {
    crate::config::load_json(&manifest_path(ch))
}

pub fn save_bank(ch: &Channel, bank: &[Clip]) -> Result<()> {
    crate::config::save_json(&manifest_path(ch), bank)
}

/// O clipe e do personagem? Casa qualquer parte do nome (>= 3 letras) na descricao, tags ou titulo da fonte.
/// Trechos do clipe por frame catalogado: onde ha outro personagem (proibido no edit de personagem unico) e
/// onde ha rosto/boca (proibido sob fala so-audio). Vazio = clipe sem catalogo por frame.
fn frame_notes(c: &Clip, character: Option<&str>) -> String {
    let mut out = String::new();
    for (a, b, f) in c.frame_spans() {
        let others: Vec<&str> = f
            .chars
            .iter()
            .filter(|x| !unknown_person(x) && character.is_none_or(|n| !same_person(x, n)))
            .map(|x| x.as_str())
            .collect();
        let mut bits = vec![];
        if character.is_some() && !others.is_empty() {
            bits.push(format!("PROIBIDO (aparece {})", others.join(", ")));
        }
        if !f.faces.is_empty() {
            bits.push(format!("rosto de {}", f.faces.join(", ")));
        }
        if bits.is_empty() {
            bits.push("sem rosto".into());
        }
        out.push_str(&format!(" [{a:.1}-{b:.1}s: {}]", bits.join("; ")));
    }
    if out.is_empty() { out } else { format!(" | trechos:{out}") }
}

/// Imagem do clipe serve ao edit do personagem? Com catalogo por frame vale quem aparece de fato (o titulo da
/// fonte "Gojo vs Toji" punha clipe so do Gojo no edit do Toji); sem ele, o texto (`text_mentions`).
pub fn shows_character(c: &Clip, character: Option<&str>, known: &[String]) -> bool {
    let Some(name) = character else { return true };
    if c.frames.iter().any(|f| f.chars.iter().any(|x| !unknown_person(x))) {
        // alguem identificado: precisa ser o personagem em algum frame
        return c.frames.iter().any(|f| f.chars.iter().any(|x| same_person(x, name)));
    }
    // sem catalogo, ou so figuras nao identificadas (costas, silhueta): vale o texto
    text_mentions(c, name, known)
}

/// Falas do clipe servem ao personagem? Catalogo primeiro: com quem fala identificado (`speaker`) vale ele;
/// sem isso, o texto ou o personagem visivel em algum frame (quem fala pode estar fora do quadro).
pub fn is_character(c: &Clip, character: Option<&str>, known: &[String]) -> bool {
    let Some(name) = character else { return true };
    let speakers: Vec<&str> = c.speech.iter().filter_map(|l| l["speaker"].as_str()).filter(|s| !unknown_person(s)).collect();
    if !speakers.is_empty() {
        return speakers.iter().any(|s| same_person(s, name));
    }
    text_mentions(c, name, known) || c.frames.iter().any(|f| f.chars.iter().any(|x| same_person(x, name)))
}

/// O texto do clipe cita o personagem? Palavra inteira, na descricao e nas tags (o titulo da fonte so quando nao
/// ha nenhuma das duas: "Gojo vs Toji" e titulo dos dois). Vale o nome inteiro, ou uma parte que nao seja de outro
/// personagem conhecido ("Toji" sim; "Fushiguro" sozinho nao, se o canal conhece Megumi Fushiguro).
pub fn text_mentions(c: &Clip, name: &str, known: &[String]) -> bool {
    let mut text = format!("{} {}", c.desc, c.tags.join(" "));
    if text.trim().is_empty() {
        text = c.source_title.clone();
    }
    let words: std::collections::HashSet<String> = name_tokens(&text).into_iter().collect();
    let parts = name_tokens(name);
    if parts.is_empty() {
        return false;
    }
    if parts.iter().all(|p| words.contains(p)) {
        return true;
    }
    let shared = |p: &String| known.iter().any(|k| !same_person(k, name) && name_tokens(k).contains(p));
    parts.iter().any(|p| words.contains(p) && !shared(p))
}

/// Falas reais disponiveis (inclusive de clipes so-audio), para o agente de recriacao.
pub fn speech_digest(ch: &Channel, character: Option<&str>) -> String {
    let mut out = String::new();
    let known = known_names(ch);
    for c in load_bank(ch).iter().filter(|c| !c.speech.is_empty() && is_character(c, character, &known)) {
        let img = if c.usable && c.quality >= 4.0 { "imagem ok" } else { "SO AUDIO (imagem ruim)" };
        out.push_str(&format!("- {} | {:.1}s | {img} | {}\n", c.file, c.duration, c.source_title));
        for (i, l) in c.speech.iter().enumerate() {
            out.push_str(&format!(
                "    fala {i} [{}] de {} {:.2}-{:.2}s (clareza {:.2}): \"{}\"\n",
                l["lang"].as_str().unwrap_or("?"),
                l["speaker"].as_str().filter(|x| !x.is_empty()).unwrap_or("?"),
                l["start"].as_f64().unwrap_or(0.0),
                l["end"].as_f64().unwrap_or(0.0),
                l["prob"].as_f64().unwrap_or(0.0),
                l["text"].as_str().unwrap_or("")
            ));
        }
    }
    if out.is_empty() {
        "(nenhuma fala no banco — rode `tkd footage --channel <canal> --speech en`)".into()
    } else {
        out
    }
}

/// Resumo do banco para os prompts (ideate e footage).
pub fn bank_digest(ch: &Channel) -> String {
    bank_digest_for(ch, None)
}

/// Resumo do banco filtrado por personagem (so imagem utilizavel).
pub fn bank_digest_for(ch: &Channel, character: Option<&str>) -> String {
    let known = known_names(ch);
    let bank: Vec<Clip> = load_bank(ch).into_iter().filter(|c| shows_character(c, character, &known)).collect();
    if bank.is_empty() {
        return "(banco vazio)".into();
    }
    bank.iter()
        .filter(|c| c.usable && !c.tags.iter().any(|t| t == "so-audio"))
        .map(|c| {
            let speech_lines = c
                .speech
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    format!(
                        "\n    fala {i} [{}] {:.2}-{:.2}s: \"{}\"",
                        l["lang"].as_str().unwrap_or("?"),
                        l["start"].as_f64().unwrap_or(0.0),
                        l["end"].as_f64().unwrap_or(0.0),
                        l["text"].as_str().unwrap_or("")
                    )
                })
                .collect::<String>();
            let q = if c.src_height > 0 && c.src_height < 720 {
                format!(" | BAIXA RES (origem {}p)", c.src_height)
            } else if c.block.unwrap_or(0.0) > 1.5 {
                format!(" | MACROBLOCO ({:.2})", c.block.unwrap_or(0.0))
            } else {
                String::new()
            };
            format!(
                "- {}{q} | {:.1}s | mov {:.2} | nota {:.1} | {} | tags: {} | serve para: {}{}{}",
                c.file,
                c.duration,
                c.motion,
                c.quality,
                c.desc,
                c.tags.join(", "),
                c.best_for,
                frame_notes(c, character),
                speech_lines
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// O YouTube barra o client web anonimo ("Sign in to confirm you're not a bot")
/// e os cookies do Edge/Chrome nao abrem mais (DPAPI app-bound).
/// `default` primeiro: e o unico que libera 1080p+ sem cookies (2026-11); `web_safari` (HLS) e o reserva.
/// Eram 5 clients; em 17/11/2026 cada busca falha virava 5 chamadas seguidas (o "format is not available" vinha
/// do desafio JS sem o yt-dlp-ejs) e ajudou a barrar o IP. `android` so entrega 360p (formato 18), que a regra de
/// 720p descarta de qualquer jeito; `ios`/`tv` quase nunca salvavam. No maximo 2 tentativas por chamada.
const CLIENTS: [&str; 2] = ["default", "web_safari"];

pub async fn ytdlp_public(args: &[&str]) -> Result<String> {
    ytdlp(args).await
}

/// Opcoes comuns do yt-dlp: runtime JS (node), pausas entre requisicoes (yt_pace), ffmpeg e, se houver, os cookies
/// da conta Google descartavel (`tkd yt-login`).
fn yt_extras(cmd: &mut Command) {
    if has_node() {
        cmd.arg("--js-runtimes").arg("node");
    }
    if let Some(c) = yt_cookies_file() {
        cmd.arg("--cookies").arg(c);
    }
    cmd.args(crate::yt_pace::SLEEP_ARGS);
    if let Ok(ff) = std::env::var("TKD_FFMPEG") {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
}

/// Gerador de PO token (plugin bgutil-ytdlp-pot-provider 2.0.2, modo script): o yt-dlp acha sozinho em
/// %USERPROFILE%/bgutil-ytdlp-pot-provider/server e chama o node a cada video. Reduz o "not a bot",
/// mas nao fura bloqueio de IP ja ativo (429). Sem ele o yt-dlp segue funcionando, so bloqueia mais.
fn warns_without_pot() {
    static ONCE_INIT: std::sync::Once = std::sync::Once::new();
    ONCE_INIT.call_once(|| {
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let js = Path::new(&home).join("bgutil-ytdlp-pot-provider").join("server").join("build").join("generate_once.js");
        if !js.exists() {
            tracing::warn!("gerador de PO token ausente ({}): o YouTube vai bloquear mais rapido", js.display());
        }
    });
}

/// node no PATH (checado uma vez).
fn has_node() -> bool {
    static NODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NODE.get_or_init(|| {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// Peso (requisicoes ao YouTube) de uma chamada, pelo tipo.
fn call_weight(args: &[&str]) -> u32 {
    use crate::yt_pace::*;
    if args.iter().any(|a| a.starts_with("ytsearch")) {
        if args.contains(&"--flat-playlist") {
            WEIGHT_SEARCH_FLAT
        } else {
            WEIGHT_SEARCH
        }
    } else if args.contains(&"--write-comments") {
        WEIGHT_META
    } else {
        WEIGHT_DOWNLOAD
    }
}

/// cookies.txt da conta Google descartavel (browser/yt_cookies.mjs). Nunca a conta de um canal: o YouTube pode
/// barrar a conta usada pelo yt-dlp. `TKD_YT_COOKIES=0` desliga.
pub fn yt_cookies_file() -> Option<PathBuf> {
    if std::env::var("TKD_YT_COOKIES").is_ok_and(|v| v == "0") {
        return None;
    }
    let f = crate::config::root().join(".profiles").join("_youtube").join("cookies.txt");
    f.exists().then_some(f)
}

/// O YouTube gira os cookies de sessao com o uso: arquivo com mais de 12 h e reexportado do perfil _youtube
/// (Edge fora da tela, trava do navegador). Uma tentativa a cada 30 min no maximo; falha so avisa.
async fn refresh_yt_cookies() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_TRY: AtomicU64 = AtomicU64::new(0);
    let Some(f) = yt_cookies_file() else { return };
    let age = std::fs::metadata(&f)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if age < 12 * 3600 || now.saturating_sub(LAST_TRY.load(Ordering::Relaxed)) < 30 * 60 {
        return;
    }
    LAST_TRY.store(now, Ordering::Relaxed);
    let r: Result<serde_json::Value> =
        crate::browser::run("yt_cookies.mjs", &serde_json::json!({ "mode": "export", "profile": "_youtube" })).await;
    match r {
        Ok(v) if v["ok"].as_bool() == Some(true) => tracing::info!("cookies do YouTube reexportados ({} cookies)", v["n"]),
        Ok(v) => tracing::warn!("cookies do YouTube: {}", v["error"].as_str().unwrap_or("falha ao exportar")),
        Err(e) => tracing::warn!("cookies do YouTube: {e:#}"),
    }
}

/// Login da conta Google descartavel no perfil _youtube e export dos cookies (`tkd yt-login`).
pub async fn yt_login(timeout_min: u64) -> Result<serde_json::Value> {
    let v: serde_json::Value = crate::browser::run(
        "yt_cookies.mjs",
        &serde_json::json!({ "mode": "login", "timeoutMin": timeout_min, "profile": "_youtube" }),
    )
    .await?;
    anyhow::ensure!(v["ok"].as_bool() == Some(true), "login do YouTube falhou: {}", v["error"].as_str().unwrap_or("?"));
    Ok(v)
}

/// Teto de uma chamada do yt-dlp (busca, metadados ou download de um video).
const YTDLP_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

async fn ytdlp(args: &[&str]) -> Result<String> {
    warns_without_pot();
    refresh_yt_cookies().await;
    let mut latest = String::new();
    for client in CLIENTS {
        crate::yt_pace::reserve(call_weight(args)).await?;
        let arg = format!("youtube:player_client={client}");
        let mut cmd = Command::new("yt-dlp");
        cmd.args(args).arg("--extractor-args").arg(&arg);
        // YouTube exige resolver desafio JS; o yt-dlp so procura deno por padrao, mas node serve
        yt_extras(&mut cmd);
        // teto: yt-dlp travado (rede caiu sem fechar a conexao) segurava o footage para sempre
        let out = match cmd.tree_output_timeout(YTDLP_LIMIT).await {
            Ok(o) => o,
            // rede presa: a mensagem antiga mandava instalar o yt-dlp
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                anyhow::bail!("yt-dlp parado sem receber nada ({e}): rede caiu ou o servidor travou; tente de novo")
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(e).context("yt-dlp nao encontrado (instale com `pip install yt-dlp`)")
            }
            Err(e) => return Err(e).context("executando yt-dlp"),
        };
        if out.status.success() {
            tracing::debug!("yt-dlp via client {client}");
            return Ok(String::from_utf8_lossy(&out.stdout).to_string());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        latest = err
            .lines()
            .filter(|l| l.contains("ERROR"))
            .last()
            .unwrap_or_else(|| err.lines().last().unwrap_or(""))
            .to_string();
        // bloqueio por excesso: trocar de client so gasta mais requisicoes e estende o bloqueio
        if crate::yt_pace::is_block(&err) {
            crate::yt_pace::mark_blocked();
            break;
        }
        // erro de conteudo (video indisponivel, sem resultado) nao melhora trocando client
        if !latest.contains("Sign in")
            && !latest.contains("format is not available")
            && !latest.contains("reloaded")
        {
            break;
        }
    }
    anyhow::bail!("yt-dlp falhou: {latest}")
}

/// Resultado de busca do YouTube lido da pagina de resultados (--flat-playlist).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    /// segundos; None quando a busca nao trouxe (live, premiere)
    pub duration: Option<f64>,
}

impl SearchHit {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

/// Busca flat no YouTube (so a pagina de resultados, peso WEIGHT_SEARCH_FLAT).
pub async fn search(query: &str, n: usize) -> Result<Vec<SearchHit>> {
    let goal = format!("ytsearch{n}:{query}");
    Ok(parse_search(&ytdlp(&[&goal, "--flat-playlist", "--print", "%(id)s|%(duration)s|%(title)s"]).await?))
}

/// Linhas `id|duracao|titulo` do `--print` da busca flat. O titulo pode ter `|`.
fn parse_search(out: &str) -> Vec<SearchHit> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.trim().splitn(3, '|');
            let id = it.next()?.trim().to_string();
            let duration = it.next()?.trim().parse::<f64>().ok();
            let title = it.next().unwrap_or("").trim().to_string();
            (id.len() == 11).then_some(SearchHit { id, title, duration })
        })
        .collect()
}

/// id de 11 caracteres de uma URL do YouTube (watch?v=, youtu.be/, shorts/).
pub fn youtube_id(url: &str) -> Option<String> {
    let rest = url
        .split_once("v=")
        .map(|(_, r)| r)
        .or_else(|| url.split_once("youtu.be/").map(|(_, r)| r))
        .or_else(|| url.split_once("/shorts/").map(|(_, r)| r))?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (id.len() == 11).then_some(id)
}

/// Fontes que nao valem um novo download: ja recortadas no banco do canal ou descartadas antes (< 720p).
fn used_sources(ch: &Channel) -> std::collections::HashSet<String> {
    let mut s: std::collections::HashSet<String> =
        load_bank(ch).iter().filter_map(|c| youtube_id(&c.source)).collect();
    s.extend(rejected_sources());
    s
}

fn rejected_path() -> PathBuf {
    crate::config::root().join("out").join("yt_meta").join("rejected.json")
}

fn rejected_sources() -> Vec<String> {
    std::fs::read_to_string(rejected_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Fonte descartada (< 720p): nao baixar de novo em buscas futuras (vale para todos os canais).
fn mark_rejected(id: &str) {
    let mut v = rejected_sources();
    if !v.iter().any(|x| x == id) {
        v.push(id.to_string());
        let p = rejected_path();
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(p, serde_json::to_string(&v).unwrap_or_default());
    }
}

/// Erro do yt-dlp de "nenhum formato casa com o filtro -f" (fonte sem 720p+). Nao confundir com o desafio JS
/// sem solucionador, que da a mesma frase junto de "Sign in"/"challenge": esse e problema nosso, nao da fonte.
fn no_hd_format(err: &str) -> bool {
    err.contains("Requested format is not available") && !err.contains("Sign in") && !err.contains("challenge")
}

/// Titulo de video com legenda queimada na imagem ("English Sub", "legendado", "vostfr"...). Legenda nao sai no
/// download: faz parte do video. "no subs"/"without subtitles"/"raw" sao o contrario e passam.
fn subbed_title(title: &str) -> bool {
    let t = title.to_lowercase();
    if ["no sub", "nosub", "without sub", "sem legenda", "no caption", "raw"].iter().any(|k| t.contains(k)) {
        return false;
    }
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    words.iter().any(|w| {
        matches!(*w, "sub" | "subs" | "subbed" | "subtitle" | "subtitles" | "subtitled" | "engsub" | "legendado"
            | "legendada" | "legenda" | "legendas" | "vostfr" | "subtitulado" | "hardsub" | "cc")
    })
}

/// Fonte limpa primeiro: "raw"/"no subs"/dublagem (sem legenda na imagem) antes do resto.
fn clean_rank(title: &str) -> u8 {
    let t = title.to_lowercase();
    if ["raw", "no sub", "without sub", "sem legenda", "dub", "dublado"].iter().any(|k| t.contains(k)) { 0 } else { 1 }
}

/// Melhor resultado dentro do limite de duracao que ainda nao foi usado e nao tem legenda no titulo.
fn pick_hit(hits: &[SearchHit], max_s: f64, used: &std::collections::HashSet<String>) -> Option<SearchHit> {
    hits.iter()
        .filter(|h| h.duration.is_some_and(|d| d > 0.0 && d < max_s))
        .filter(|h| !used.contains(&h.id) && !subbed_title(&h.title))
        .min_by_key(|h| clean_rank(&h.title))
        .cloned()
}

/// Baixa o melhor resultado ainda nao usado da busca (sem audio: so precisamos da imagem).
async fn download_video(
    query: &str,
    max_minutes: u64,
    dst: &Path,
    used: &std::collections::HashSet<String>,
) -> Result<(String, String, Option<PathBuf>)> {
    let out_str = dst.to_string_lossy().to_string();

    // busca flat: so a pagina de resultados (~2 requisicoes; a busca normal abria cada video, ~6).
    // 6 resultados: o 1o costuma ser uma fonte ja recortada (as buscas do agente se repetem entre edits).
    let hits = search(query, 6).await?;
    anyhow::ensure!(!hits.is_empty(), "nenhum resultado para \"{query}\"");
    let hit = pick_hit(&hits, (max_minutes * 60) as f64, used).ok_or_else(|| {
        anyhow::anyhow!("\"{query}\": os {} resultados ja estao no banco, foram descartados, tem legenda no titulo ou passam de {max_minutes} min", hits.len())
    })?;
    let url = hit.url();

    // cobalt na frente (baixa com o IP da instancia, fora do orcamento do YouTube); sem instancia util, o yt-dlp assume
    match crate::cobalt::download_video(&url, dst).await {
        Ok(_) => tracing::info!("baixado pelo cobalt (fora do orcamento do YouTube): {}", hit.title),
        Err(e) => {
            tracing::info!("{e:#}; baixando pelo yt-dlp");
            if let Err(e) = ytdlp(&[
                &url, "--no-playlist", "-f", "bv*[height>=1080][height<=2160]+ba/bv*[height>=720][height<=2160]+ba/b[height>=720]",
                "--merge-output-format", "mp4", "-o", &out_str, "--no-progress", "--quiet",
            ])
            .await
            {
                // sem formato >= 720p (o filtro -f nao casa): mesma regra do probe, nao tentar essa fonte de novo
                if no_hd_format(&format!("{e:#}")) {
                    mark_rejected(&hit.id);
                }
                return Err(e);
            }
        }
    }
    anyhow::ensure!(dst.exists(), "download nao gerou arquivo");
    // regra de qualidade: nunca recortar de fonte abaixo de 720p (docs/QUALITY_RULES.md)
    let h = media::probe(dst).await.map(|p| p.height).unwrap_or(0);
    if h < 720 {
        mark_rejected(&hit.id);
        anyhow::bail!("fonte so tem {h}p (minimo 720p) — descartada");
    }
    let meta = download_meta(&url, dst).await;
    Ok((url, hit.title, meta))
}

/// Cache da meta por id de video: comentarios e heatmap mudam pouco e a meta e a chamada mais cara (peso 12).
fn meta_cache(id: &str) -> PathBuf {
    crate::config::root().join("out").join("yt_meta").join(format!("{id}.info.json"))
}

/// info.json do YouTube com heatmap ("mais repetidos") e comentarios — sem login. Opcional: falha = None.
/// Devolve uma copia em `<dst>-meta.info.json` (o chamador apaga); o original fica no cache por id.
pub async fn download_meta(url: &str, dst: &Path) -> Option<PathBuf> {
    let stem = dst.with_file_name(format!(
        "{}-meta",
        dst.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let goal = PathBuf::from(format!("{}.info.json", stem.to_string_lossy()));
    let _ = std::fs::remove_file(&goal);
    let cache = youtube_id(url).map(|id| meta_cache(&id));
    if let Some(c) = cache.as_ref().filter(|c| c.exists()) {
        if std::fs::copy(c, &goal).is_ok() {
            tracing::info!("meta do YouTube do cache ({}): 0 requisicoes", c.display());
            return Some(goal);
        }
    }
    if let Err(e) = crate::yt_pace::reserve(crate::yt_pace::WEIGHT_META).await {
        tracing::warn!("meta do YouTube pulada: {e}");
        return None;
    }
    // um unico --extractor-args: o yt-dlp nao junta dois do mesmo extrator (o 2o apaga o max_comments)
    // 200 comentarios (~10 paginas) bastam para os timestamps citados e cabem no orcamento por hora.
    // max_replies=0: respostas ocupavam vaga dos comentarios de 1o nivel (onde ficam os horarios) e cada
    // fio com respostas custava requisicoes a mais.
    let mut cmd = Command::new("yt-dlp");
    cmd.args([
        url, "--no-playlist", "--skip-download", "--write-info-json", "--write-comments",
        "--extractor-args", "youtube:max_comments=200,all,0;player_client=default",
        "-o", &stem.to_string_lossy(), "--no-progress", "--quiet",
    ]);
    yt_extras(&mut cmd);
    cmd.kill_on_drop(true);
    let r = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.tree_output()).await;
    match r {
        Ok(Ok(_)) if goal.exists() => {
            if let Some(c) = &cache {
                let _ = std::fs::create_dir_all(c.parent().unwrap_or(Path::new(".")));
                let _ = std::fs::copy(&goal, c);
            }
            Some(goal)
        }
        Ok(Ok(o)) => {
            let err = String::from_utf8_lossy(&o.stderr);
            if crate::yt_pace::is_block(&err) {
                crate::yt_pace::mark_blocked();
            }
            tracing::warn!(
                "meta do YouTube indisponivel ({}); seguindo sem comentarios/heatmap",
                err.lines().last().unwrap_or("")
            );
            None
        }
        Ok(Err(e)) => {
            tracing::warn!("yt-dlp nao rodou para meta ({e})");
            None
        }
        Err(_) => {
            tracing::warn!("meta do YouTube passou de 120 s; seguindo sem comentarios/heatmap");
            None
        }
    }
}

/// Recorta os melhores planos do video baixado, no formato do canal.
async fn cut_clip(
    source_path: &Path,
    ch: &Channel,
    aspect_ratio: (u32, u32),
    max_clips: usize,
    prefix: &str,
    mom: &MomCtx,
    subs_top: Option<f64>,
) -> Result<Vec<(PathBuf, f64, f64, f64, f64)>> {
    // tres leituras independentes da fonte inteira: juntas em vez de uma depois da outra (cada decode e ~1 nucleo)
    let (curve_pts, detector, probe) =
        tokio::join!(media::motion_curve(source_path), find_moments(source_path, mom, None, max_clips.max(4)), media::probe(source_path));
    let (curve_pts, probe) = (curve_pts?, probe?);
    let transition_list = media::detect_transitions(&curve_pts);

    // fronteiras de plano a partir das transicoes detectadas
    let mut borders = vec![0.0];
    borders.extend(transition_list.iter().map(|t| t.t));
    borders.push(probe.duration);

    let mut shortlist: Vec<(f64, f64, f64)> = Vec::new(); // (inicio, duracao, movimento)
    for par in borders.windows(2) {
        let (seg_start, end_at) = (par[0], par[1]);
        let dur = end_at - seg_start;
        if !(1.6..=8.0).contains(&dur) {
            continue;
        }
        // descarta os 8% iniciais e finais (intro/outro de canal)
        if seg_start < probe.duration * 0.08 || end_at > probe.duration * 0.92 {
            continue;
        }
        let motion_v: f64 = {
            let win: Vec<f64> = curve_pts
                .iter()
                .filter(|(t, _)| *t > seg_start + 0.15 && *t < end_at - 0.15)
                .map(|(_, s)| *s)
                .collect();
            if win.is_empty() {
                0.0
            } else {
                win.iter().sum::<f64>() / win.len() as f64
            }
        };
        shortlist.push((seg_start + 0.12, (dur - 0.3).min(6.0).max(1.4), motion_v));
    }

    // nota = momento forte da fonte (detector) - distancia do movimento ideal (nem parado, nem tremendo).
    // Sem detector: so o movimento, como antes.
    let moment_score =
        |c: &(f64, f64, f64)| detector.as_ref().map(|d| d.rating(c.0, c.0 + c.1)).unwrap_or(0.0);
    let rating = |c: &(f64, f64, f64)| moment_score(c) - 4.0 * (c.2 - 0.05).abs();
    shortlist.sort_by(|a, b| rating(b).partial_cmp(&rating(a)).unwrap_or(std::cmp::Ordering::Equal));
    shortlist.truncate(max_clips);
    shortlist.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let dir = clips_dir(ch);
    std::fs::create_dir_all(&dir)?;
    let (aw, ah) = aspect_ratio;
    let bars = bars_filter(source_path, probe.duration, subs_top).await;
    let mut cut_files = Vec::new();
    for (i, c) in shortlist.into_iter().enumerate() {
        let mom_item = moment_score(&c);
        let (seg_start, dur, motion_v) = c;
        let dst = dir.join(format!("{prefix}-{i:02}.mp4"));
        // corta no centro para o formato do canal e normaliza em 30fps
        let vf = format!(
            "{bars}crop='min(iw,ih*{aw}/{ah})':'min(ih,iw*{ah}/{aw})',scale={aw_px}:{ah_px}:flags=lanczos,fps=30",
            aw_px = if aw == ah { 1080 } else { 1080 },
            ah_px = 1080 * ah / aw,
        );
        let status = Command::new(crate::media::ffmpeg_bin())
            .args([
                "-y", "-v", "error",
                "-ss", &format!("{seg_start:.2}"), "-t", &format!("{dur:.2}"),
                "-i", &source_path.to_string_lossy(),
                "-vf", &vf,
                // o audio original do clipe e parte do edit (fala do personagem)
                "-c:a", "aac", "-b:a", "192k", "-ac", "2",
                "-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p",
                &dst.to_string_lossy(),
            ])
            .tree_status()
            .await?;
        if status.success() && dst.exists() {
            cut_files.push((dst, seg_start, dur, motion_v, mom_item));
        } else {
            // ffmpeg morto no meio deixa mp4 pela metade na pasta de clipes
            let _ = std::fs::remove_file(&dst);
        }
    }
    Ok(cut_files)
}

/// Modo falas: recorta frases inteiras (com audio) em vez de planos de camera.
/// Devolve (arquivo, inicio na fonte, duracao, falas com tempo relativo ao recorte).
async fn cut_speech(
    source_path: &Path,
    ch: &Channel,
    aspect_ratio: (u32, u32),
    max_clips: usize,
    prefix: &str,
    lang: &str,
    mom: &MomCtx,
    subs_top: Option<f64>,
    hotwords: &str,
) -> Result<Vec<(PathBuf, f64, f64, Vec<serde_json::Value>, f64)>> {
    let seg_json = std::env::temp_dir().join(format!("{prefix}-segs.json"));
    let _seg_cleanup = crate::proc::RemoveOnDrop(seg_json.clone());
    let st = Command::new(crate::recreate::venv_bin("python"))
        .args(["-I", "-X", "utf8", "scripts/speech.py", "segments", &source_path.to_string_lossy(), &seg_json.to_string_lossy(), "--lang", lang])
        .args(["--hotwords", hotwords])
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .current_dir(crate::config::root())
        .tree_status()
        .await
        .context("scripts/speech.py segments")?;
    anyhow::ensure!(st.success(), "speech.py segments falhou ({st})");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&seg_json)?)?;
    // detector reaproveita as falas ja transcritas (nao roda o whisper de novo)
    let detector = find_moments(source_path, mom, Some(&seg_json), max_clips.max(4)).await;
    let _ = std::fs::remove_file(&seg_json);
    let mut segs: Vec<serde_json::Value> = v["segments"].as_array().cloned().unwrap_or_default();
    let clarity = |s: &serde_json::Value| {
        let ls = s["lines"].as_array().cloned().unwrap_or_default();
        ls.iter().map(|l| l["prob"].as_f64().unwrap_or(0.0)).sum::<f64>() / ls.len().max(1) as f64
    };
    let moment_score = |s: &serde_json::Value| {
        detector
            .as_ref()
            .map(|d| d.rating(s["start"].as_f64().unwrap_or(0.0), s["end"].as_f64().unwrap_or(0.0)))
            .unwrap_or(0.0)
    };
    // momento normalizado 0..1 entre os segmentos desta fonte
    let ratings: Vec<f64> = segs.iter().map(&moment_score).collect();
    let (lo, hi) = ratings.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    let norm = |x: f64| if hi - lo > 1e-9 { (x - lo) / (hi - lo) } else { 0.0 };
    // falas claras E em momento forte primeiro (sem detector: so clareza), depois volta a ordem do video
    let score = |s: &serde_json::Value| {
        if detector.is_some() {
            0.5 * clarity(s) + 0.5 * norm(moment_score(s))
        } else {
            clarity(s)
        }
    };
    segs.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal));
    segs.truncate(max_clips);
    segs.sort_by(|a, b| a["start"].as_f64().partial_cmp(&b["start"].as_f64()).unwrap());

    let dir = clips_dir(ch);
    std::fs::create_dir_all(&dir)?;
    let (aw, ah) = aspect_ratio;
    let src_dur = media::probe(source_path).await.map(|p| p.duration).unwrap_or(0.0);
    let bars = bars_filter(source_path, src_dur, subs_top).await;
    let vf = format!(
        "{bars}crop='min(iw,ih*{aw}/{ah})':'min(ih,iw*{ah}/{aw})',scale=1080:{h}:flags=lanczos,fps=30",
        h = 1080 * ah / aw
    );
    let mut cut_files = Vec::new();
    for (i, sg) in segs.iter().enumerate() {
        let seg_start = (sg["start"].as_f64().unwrap_or(0.0) - 0.25).max(0.0);
        let dur = sg["end"].as_f64().unwrap_or(0.0) + 0.4 - seg_start;
        let dst = dir.join(format!("{prefix}-f{i:02}.mp4"));
        let ok = Command::new(crate::media::ffmpeg_bin())
            .args([
                "-y", "-v", "error", "-ss", &format!("{seg_start:.3}"), "-t", &format!("{dur:.3}"),
                "-i", &source_path.to_string_lossy(), "-vf", &vf,
                "-c:a", "aac", "-b:a", "192k", "-ac", "2",
                "-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p",
                &dst.to_string_lossy(),
            ])
            .tree_status()
            .await?
            .success();
        if !ok || !dst.exists() {
            let _ = std::fs::remove_file(&dst);
            continue;
        }
        // tempos das falas passam a ser relativos ao recorte
        let sh = |x: &serde_json::Value| serde_json::json!(((x.as_f64().unwrap_or(0.0) - seg_start) * 1000.0).round() / 1000.0);
        let speech_lines = sg["lines"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut l| {
                l["start"] = sh(&l["start"]);
                l["end"] = sh(&l["end"]);
                if let Some(ws) = l["words"].as_array_mut() {
                    for w in ws {
                        w["s"] = sh(&w["s"]);
                        w["e"] = sh(&w["e"]);
                    }
                }
                l
            })
            .collect();
        cut_files.push((dst, seg_start, dur, speech_lines, moment_score(sg)));
    }
    Ok(cut_files)
}

pub(crate) fn channel_aspect(ch: &Channel) -> (u32, u32) {
    let preset = std::fs::read_to_string(ch.preset()).unwrap_or_default();
    let a = serde_json::from_str::<serde_json::Value>(&preset)
        .ok()
        .and_then(|p| p["aspect"].as_str().map(String::from))
        .unwrap_or_else(|| "9:16".into());
    match a.as_str() {
        "1:1" => (1, 1),
        "4:5" => (4, 5),
        "4:3" => (4, 3),
        _ => (9, 16),
    }
}

/// Busca, baixa e recorta footage nova para o canal; depois cataloga com visao.
pub async fn fetch(
    ch: &Channel,
    store: &Store,
    topic: Option<&str>,
    n_queries: usize,
    per_search: usize,
    speech_lines: Option<&str>,
    character: Option<&str>,
) -> Result<usize> {
    let span = crate::events::step(&ch.slug, "footage");
    span.action("Buscando imagens", "python");
    let r = fetch_inner(ch, store, topic, n_queries, per_search, speech_lines, character).await;
    span.finish(r, |n| format!("{n} clipes novos"))
}

async fn fetch_inner(
    ch: &Channel,
    store: &Store,
    topic: Option<&str>,
    n_queries: usize,
    per_search: usize,
    speech_lines: Option<&str>,
    character: Option<&str>,
) -> Result<usize> {
    anyhow::ensure!(
        ch.style_card().exists(),
        "{} ainda não estudou nenhuma referência: adicione um canal pra estudar e clique em Mandar estudar          (na CLI: `tkd study --channel {}`)",
        ch.slug,
        ch.slug
    );

    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("style_card", agent::style_card_visual(ch)?);
    vars.insert("bank", bank_digest(ch));
    let mut theme_word = topic
        .unwrap_or("o que faltar para cobrir o estilo do canal")
        .to_string();
    if let Some(p) = character {
        theme_word.push_str(&format!(
            "\n\nPERSONAGEM UNICO: `{p}`. TODAS as buscas devem ser cenas em que {p} e o foco (aparece e/ou fala). \
             Nada de outros personagens como foco. Ponha o nome dele em cada busca."
        ));
    }
    if let Some(l) = speech_lines {
        theme_word.push_str(&format!(
            "\n\nMODO FALAS: o objetivo e a VOZ do personagem. Procure cenas de dialogo/monologo com a fala \
             clara e pouca musica por cima (cena isolada: 'scene', 'monologue', 'speech'), no idioma `{l}` \
             (en = dublagem inglesa ou original em ingles; ja = original japones; pt = dublagem brasileira; \
             auto = qualquer). Ponha o idioma na busca (ex.: 'english dub', 'dublado'). max_minutes <= 15.              Legenda queimada na imagem estraga o clipe (duplica a nossa e a cena vira so audio): nunca busque              'sub'/'legendado'; para japones use 'raw' ou 'no subs'."
        ));
    }
    vars.insert("topic", theme_word);
    vars.insert("n_queries", n_queries.to_string());

    let prompt = agent::render_template("footage.md", &vars)?;
    let plan_out: Queries = agent::ask_json(ch, "footage", &prompt, &[], 300, 2).await?;
    let hot = hotwords(ch, &plan_out.names, character);

    media::check_ffmpeg().await;
    let aspect_ratio = channel_aspect(ch);
    let mut bank = load_bank(ch);
    let mut new_items: usize = 0;
    // falha de cada busca; se nenhuma render clipe, vira o erro (antes sumia no log e a tela so dizia "0 clipes")
    let mut download_errors: Vec<String> = Vec::new();
    let mom_base = MomCtx::for_channel(ch, speech_lines.is_some(), character);
    // fontes ja recortadas: baixar de novo so repetiria os mesmos planos e gastaria orcamento do YouTube
    let mut used = used_sources(ch);

    for (qi, q) in plan_out.queries.iter().enumerate().take(n_queries) {
        tracing::info!("[{}] buscando: {} ({})", ch.slug, q.q, q.why);
        let temp = std::env::temp_dir().join(format!("tkd-src-{}-{qi}.mp4", ch.slug));
        // fonte baixada (centenas de MB) sai do %TEMP% mesmo se o recorte falhar ou a task for cancelada
        let _temp_cleanup = crate::proc::RemoveFamily::new(temp.clone());
        let (url, title_text, meta) = match download_video(&q.q, q.max_minutes, &temp, &used).await {
            Ok(v) => {
                if let Some(id) = youtube_id(&v.0) {
                    used.insert(id);
                }
                v
            }
            Err(e) => {
                tracing::warn!("[{}] download falhou ({}): {e}", ch.slug, q.q);
                download_errors.push(format!("\"{}\": {e:#}", q.q));
                continue;
            }
        };
        tracing::info!("[{}] baixado: {title_text}", ch.slug);
        // legenda queimada (fansub/"Eng Sub") duplicaria a nossa e estraga a fala: recorta acima ou descarta
        let subs_top = match check_burned_subs(&temp).await {
            SubsCheck::Clean => None,
            SubsCheck::Crop(top) => Some(top),
            SubsCheck::Reject(why) => {
                if let Some(id) = youtube_id(&url) {
                    mark_rejected(&id);
                }
                tracing::warn!("[{}] fonte descartada: {why} ({title_text})", ch.slug);
                download_errors.push(format!("\"{}\": {why}", q.q));
                continue;
            }
        };
        let _meta_cleanup = meta.clone().map(crate::proc::RemoveOnDrop);
        let src_height = media::probe(&temp).await.map(|p| p.height).unwrap_or(0);

        let prefix = format!(
            "{}-{}",
            ch.slug,
            chrono::Utc::now().format("%m%d%H%M%S")
        );
        let mom = MomCtx { meta: meta.clone(), ..mom_base.clone() };
        type CutRow = (PathBuf, f64, f64, f64, Vec<serde_json::Value>, f64);
        let cut_rows: Vec<CutRow> = match speech_lines {
            Some(l) => cut_speech(&temp, ch, aspect_ratio, per_search, &prefix, l, &mom, subs_top, &hot)
                .await?
                .into_iter()
                .map(|(f, i, d, s, m)| (f, i, d, 0.0, s, m))
                .collect(),
            None => cut_clip(&temp, ch, aspect_ratio, per_search, &prefix, &mom, subs_top)
                .await?
                .into_iter()
                .map(|(f, i, d, mv, m)| (f, i, d, mv, vec![], m))
                .collect(),
        };
        let _ = std::fs::remove_file(&temp);
        if let Some(m) = &meta {
            let _ = std::fs::remove_file(m);
        }

        for (file, seg_start, dur, motion_v, speech, moment) in cut_rows {
            let rel = file
                .strip_prefix(public_dir())
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let palette = media::palette(&file, 4).await.unwrap_or_default();
            bank.push(Clip {
                file: rel,
                source: url.clone(),
                source_title: title_text.clone(),
                start: seg_start,
                duration: dur,
                motion: motion_v,
                palette,
                tags: q.tags.clone(),
                desc: String::new(),
                mood: String::new(),
                best_for: String::new(),
                quality: 0.0,
                usable: true,
                speech,
                src_height,
                block: None,
                moment: (moment * 1000.0).round() / 1000.0,
                frames: vec![],
            });
            new_items += 1;
        }
        save_bank(ch, &bank)?;
    }

    if new_items > 0 {
        let prior = bank.len();
        catalog_bank(ch, &mut bank).await?;
        save_bank(ch, &bank)?;
        // o catalogo descarta recortes ruins: conta so os que ficaram
        new_items = new_items.saturating_sub(prior.saturating_sub(bank.len()));
    }
    if new_items == 0 && !download_errors.is_empty() {
        let msg = format!("nenhuma busca baixou: {}", download_errors.join(" | "));
        store.log(&ch.slug, "footage", "erro", &msg)?;
        anyhow::bail!(msg);
    }
    store.log(&ch.slug, "footage", "ok", &format!("{new_items} clipes novos"))?;
    Ok(new_items)
}

/// Tag do `scripts/burned_subs.py bank`: legenda queimada medida por OCR (clipe inutilizavel para imagem).
pub const BURNED_SUBS_TAG: &str = "legenda queimada";

/// Clipes por chamada do catalogo. Com 12 clipes parecidos numa chamada o Claude "pulou uma linha" e deu a cada
/// clipe a descricao do seguinte (18/11: close do Gojo virou "dois homens de costas" e entrou no edit do Toji).
const CATALOG_BATCH: usize = 4;

#[derive(Deserialize)]
struct Tagged {
    clips: Vec<TaggedClip>,
}

#[derive(Deserialize)]
struct TaggedClip {
    file: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    mood: String,
    #[serde(default)]
    best_for: String,
    #[serde(default)]
    quality: f64,
    #[serde(default = "yes_default")]
    usable: bool,
    #[serde(default)]
    frames: Vec<TaggedFrame>,
    /// quem fala cada fala do clipe: [{"line": k, "who": "Nome"}]
    #[serde(default)]
    speakers: Vec<TaggedSpeaker>,
}

#[derive(Deserialize)]
struct TaggedSpeaker {
    line: usize,
    #[serde(default)]
    who: String,
}

#[derive(Deserialize)]
struct TaggedFrame {
    #[serde(default)]
    frame: String,
    #[serde(default)]
    chars: Vec<String>,
    #[serde(default)]
    faces: Vec<String>,
}

/// Cenas do clipe (cortes detectados; cena longa vira pedacos de <= 2.5 s), com um frame no meio de cada.
/// Fatias fixas (3 frames) nao viam um close de 1 s entre dois frames num clipe de fala de 8 s com cortes.
async fn scene_frames(file: &Path, dir: &Path, duration: f64) -> Vec<(f64, f64, PathBuf)> {
    let curve = media::motion_curve(file).await.unwrap_or_default();
    let mut edges = vec![0.0];
    edges.extend(media::detect_transitions(&curve).iter().map(|t| t.t).filter(|t| *t > 0.15 && *t < duration - 0.15));
    edges.push(duration);
    let mut spans: Vec<(f64, f64)> = vec![];
    for w in edges.windows(2) {
        let (a, b) = (w[0], w[1]);
        let k = ((b - a) / 2.5).ceil().max(1.0) as usize;
        for j in 0..k {
            spans.push((a + (b - a) * j as f64 / k as f64, a + (b - a) * (j + 1) as f64 / k as f64));
        }
    }
    // cena de 1-2 frames (flash) nao precisa de frame proprio; no maximo 10 por clipe
    spans.retain(|(a, b)| b - a >= 0.12);
    spans.truncate(10);
    let stem = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let mut out = vec![];
    for (i, (a, b)) in spans.into_iter().enumerate() {
        let raw = dir.join(format!("{stem}-s{i}.raw.png"));
        let dst = dir.join(format!("{stem}-s{i}.png"));
        if let Ok(Some(_)) = media::frame_at(file, (a + b) / 2.0, &raw, 360).await {
            label_frame(&raw, &dst, &format!("F{}", out.len())).await;
            let _ = std::fs::remove_file(&raw);
            if dst.exists() {
                out.push((a, b, dst));
            }
        }
    }
    out
}

/// Rotulo ("F3") escrito no canto do frame. O catalogo descreve o frame pelo rotulo que VE na imagem: com so a
/// ordem da lista ele pulou um flash de 0,2 s e deslocou todos os rotulos seguintes do clipe (18/11).
async fn label_frame(src: &Path, dst: &Path, label: &str) {
    let font = ["C:/Windows/Fonts/arialbd.ttf", "C:/Windows/Fonts/arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"]
        .into_iter()
        .find(|f| Path::new(f).exists());
    let ok = match font {
        Some(f) => {
            let vf = format!(
                "drawbox=x=0:y=0:w=110:h=64:color=black@0.85:t=fill,drawtext=fontfile='{}':text='{label}':x=14:y=10:fontsize=44:fontcolor=yellow",
                f.replace(':', r"\:")
            );
            Command::new(media::ffmpeg_bin())
                .args(["-v", "error", "-y", "-i", &src.to_string_lossy(), "-vf", &vf, &dst.to_string_lossy()])
                .tree_status()
                .await
                .is_ok_and(|s| s.success())
        }
        None => false,
    };
    if !ok {
        let _ = std::fs::copy(src, dst);
    }
}

/// Resposta do catalogo bate com o clipe? Cada frame vem com o nome do PNG, que carrega o nome do clipe:
/// descricao deslocada de outro clipe chega com os frames do outro e e recusada.
fn catalog_entry_ok(t: &TaggedClip, expected: &[PathBuf]) -> bool {
    t.frames.len() == expected.len()
        && t.frames.iter().enumerate().all(|(k, f)| f.frame.trim().eq_ignore_ascii_case(&format!("F{k}")))
}

/// Descreve e nota os clipes ainda sem catalogo, olhando frames de cada um (e diz quem aparece em cada frame).
/// `refresh` = refaz tambem os ja descritos que nao tem o catalogo por frame.
pub async fn catalog_bank(ch: &Channel, bank: &mut Vec<Clip>) -> Result<()> {
    catalog_bank_with(ch, bank, false).await
}

pub async fn catalog_bank_with(ch: &Channel, bank: &mut Vec<Clip>, refresh: bool) -> Result<()> {
    let frames_dir = ch.knowledge_dir().join("clipframes");
    // (indice no banco, frames); trechos de cada frame guardados a parte
    let mut pending_list: Vec<(usize, Vec<PathBuf>)> = vec![];
    let mut spans: HashMap<usize, Vec<(f64, f64)>> = HashMap::new();
    for (i, c) in bank.iter().enumerate() {
        let old_catalog = c.frames.is_empty()
            || c.frames.iter().all(|f| f.b <= f.a)
            || c.speech.iter().any(|l| l.get("speaker").is_none());
        if !(c.desc.is_empty() || (refresh && old_catalog)) {
            continue;
        }
        let abs = public_dir().join(&c.file);
        if !abs.exists() {
            continue;
        }
        let sc = scene_frames(&abs, &frames_dir, c.duration).await;
        if !sc.is_empty() {
            spans.insert(i, sc.iter().map(|(a, b, _)| (*a, *b)).collect());
            pending_list.push((i, sc.into_iter().map(|(_, _, p)| p).collect()));
        }
    }
    if pending_list.is_empty() {
        return Ok(());
    }

    let style = agent::style_card_visual(ch)?;
    let mut failures: Vec<(usize, Vec<PathBuf>)> = vec![];
    for pass_n in 0..2 {
        let current_batch = if pass_n == 0 { std::mem::take(&mut pending_list) } else { std::mem::take(&mut failures) };
        if current_batch.is_empty() {
            break;
        }
        let mut set = tokio::task::JoinSet::new();
        for chunk_batch in current_batch.chunks(CATALOG_BATCH) {
            let blocks = chunk_batch
                .iter()
                .map(|(i, fs)| {
                    let c = &bank[*i];
                    let sp = spans.get(i).cloned().unwrap_or_default();
                    let speech_lines = c
                        .speech
                        .iter()
                        .enumerate()
                        .map(|(k, l)| {
                            let (a, b) = (l["start"].as_f64().unwrap_or(0.0), l["end"].as_f64().unwrap_or(0.0));
                            let em: Vec<String> = sp
                                .iter()
                                .enumerate()
                                .filter(|(_, (x, y))| *x < b && *y > a)
                                .map(|(j, _)| format!("F{j}"))
                                .collect();
                            format!("
  - fala {k} ({a:.1}-{b:.1}s, soa durante {}): \"{}\"", em.join(", "), l["text"].as_str().unwrap_or(""))
                        })
                        .collect::<String>();
                    let speech_lines = if speech_lines.is_empty() { String::new() } else { format!("
- falas (diga quem fala cada uma em `speakers`):{speech_lines}") };
                    format!(
                        "### {}
- {:.1}s | movimento {:.3} | paleta {:?}
- frames (um por cena, rotulo F0, F1... escrito no canto de cada imagem):
{}",
                        c.file,
                        c.duration,
                        c.motion,
                        c.palette,
                        fs.iter().map(|p| format!("  - {}", p.display())).collect::<Vec<_>>().join("
")
                    ) + &speech_lines
                })
                .collect::<Vec<_>>()
                .join("

");
            let mut vars = HashMap::new();
            vars.insert("slug", ch.slug.clone());
            vars.insert("style_card", style.clone());
            vars.insert("clips", blocks);
            let prompt = agent::render_template("tag_clips.md", &vars)?;
            let ch2 = ch.clone();
            let chunk_batch = chunk_batch.to_vec();
            set.spawn(async move {
                let r: Result<Tagged> = agent::ask_json(&ch2, "tag_clips", &prompt, &["Read"], 600, 2).await;
                (chunk_batch, r)
            });
        }
        while let Some(j) = set.join_next().await {
            let (chunk_batch, r) = j?;
            let out = match r {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("[{}] catalogo: lote falhou ({e:#})", ch.slug);
                    failures.extend(chunk_batch);
                    continue;
                }
            };
            for (i, fs) in chunk_batch {
                let file = bank[i].file.clone();
                let Some(t) = out.clips.iter().find(|t| t.file == file) else {
                    failures.push((i, fs));
                    continue;
                };
                if !catalog_entry_ok(t, &fs) {
                    tracing::warn!("[{}] catalogo: resposta de {file} nao bate com os frames dele; refazendo", ch.slug);
                    failures.push((i, fs));
                    continue;
                }
                let c = &mut bank[i];
                // marcas de deteccao (OCR de legenda queimada, so-audio) sao fato medido: o catalogo nao desfaz
                let keep: Vec<String> =
                    c.tags.iter().filter(|t| *t == BURNED_SUBS_TAG || *t == "so-audio").cloned().collect();
                c.desc = t.desc.clone();
                if !t.tags.is_empty() {
                    c.tags = t.tags.clone();
                }
                for k in keep {
                    if !c.tags.contains(&k) {
                        c.tags.push(k);
                    }
                }
                c.mood = t.mood.clone();
                c.best_for = t.best_for.clone();
                c.quality = t.quality;
                c.usable = t.usable && !c.tags.iter().any(|t| t == BURNED_SUBS_TAG);
                for spk in &t.speakers {
                    if let Some(l) = c.speech.get_mut(spk.line) {
                        l["speaker"] = serde_json::json!(spk.who.trim());
                    }
                }
                let sp = spans.get(&i).cloned().unwrap_or_default();
                let r2 = |x: f64| (x * 100.0).round() / 100.0;
                c.frames = t
                    .frames
                    .iter()
                    .zip(sp)
                    .map(|(f, (a, b))| FrameTag {
                        t: r2((a + b) / 2.0),
                        a: r2(a),
                        b: r2(b),
                        chars: f.chars.clone(),
                        faces: f.faces.clone(),
                    })
                    .collect();
            }
        }
        if pass_n == 0 && !failures.is_empty() {
            tracing::info!("[{}] catalogo: refazendo {} clipes", ch.slug, failures.len());
        }
    }
    if !failures.is_empty() {
        tracing::warn!("[{}] catalogo: {} clipes sem catalogo confiavel (ficam pendentes)", ch.slug, failures.len());
    }

    // descarta o que o catalogo reprovou
    let prior = bank.len();
    for c in bank.iter_mut() {
        // a VOZ continua valendo mesmo com imagem ruim (legenda queimada, logo): vira clipe so-audio
        if !(c.usable && c.quality >= 4.0) && !c.speech.is_empty() && !c.tags.iter().any(|t| t == "so-audio") {
            c.tags.push("so-audio".into());
        }
    }
    // recatalogo (--refresh) so marca: nunca apaga clipe que ja estava no banco
    bank.retain(|c| {
        if refresh || (c.usable && c.quality >= 4.0) || !c.speech.is_empty() {
            true
        } else {
            let _ = std::fs::remove_file(public_dir().join(&c.file));
            false
        }
    });
    tracing::info!(
        "[{}] catalogo: {} clipes, {} descartados",
        ch.slug,
        bank.len(),
        prior - bank.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_lines() {
        let out = "abcdefghijk|612.0|Toji vs Gojo | Full Fight\nzzzzzzzzzzz|NA|Live agora\nbad|10|x\n";
        let h = parse_search(out);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].title, "Toji vs Gojo | Full Fight");
        assert_eq!(h[0].duration, Some(612.0));
        assert_eq!(h[1].duration, None);
    }

    #[test]
    fn skips_used_and_long() {
        let hit = |id: &str, d: f64| SearchHit { id: id.into(), title: String::new(), duration: Some(d) };
        let hits = vec![hit("aaaaaaaaaaa", 300.0), hit("bbbbbbbbbbb", 9000.0), hit("ccccccccccc", 200.0)];
        let used: std::collections::HashSet<String> = ["aaaaaaaaaaa".to_string()].into();
        assert_eq!(pick_hit(&hits, 600.0, &used).unwrap().id, "ccccccccccc");
        let all: std::collections::HashSet<String> = ["aaaaaaaaaaa".into(), "ccccccccccc".into()].into();
        assert!(pick_hit(&hits, 600.0, &all).is_none());
    }

    #[test]
    fn catalog_by_frame() {
        let f = |chars: &[&str], faces: &[&str]| FrameTag {
            t: 0.0,
            a: 0.0,
            b: 0.0,
            chars: chars.iter().map(|s| s.to_string()).collect(),
            faces: faces.iter().map(|s| s.to_string()).collect(),
        };
        let mut c: Clip = serde_json::from_str(r#"{"file":"x.mp4","source":"","source_title":"Gojo vs Toji","start":0,"duration":3.0,"motion":0}"#).unwrap();
        c.frames = vec![f(&["Satoru Gojo"], &["Satoru Gojo"]), f(&[], &[]), f(&["Toji Fushiguro"], &["Toji Fushiguro"])];
        // trecho explicito (catalogo por cena) vale no lugar da fatia igual
        let mut d = c.clone();
        d.frames[0].a = 0.0;
        d.frames[0].b = 0.3;
        assert!(d.others_in(0.35, 0.9, "Toji").is_empty());
        assert!(shows_character(&c, Some("Toji Fushiguro"), &[]));
        assert_eq!(c.others_in(0.0, 0.9, "Toji"), vec!["Satoru Gojo".to_string()]);
        assert!(c.others_in(1.05, 2.9, "Toji").is_empty());
        assert!(c.face_in(1.05, 1.95).is_none());
        assert_eq!(c.face_in(2.1, 2.9).as_deref(), Some("Toji Fushiguro"));
        // so o Gojo identificado: fora do edit do Toji, mesmo com "Toji" no titulo da fonte
        c.frames = vec![f(&["Satoru Gojo"], &[]), f(&["desconhecido"], &[])];
        assert!(!shows_character(&c, Some("Toji Fushiguro"), &[]));
        // so figuras de costas nao identificadas: neutras
        c.frames = vec![f(&["desconhecido"], &[]), f(&[], &[])];
        assert!(shows_character(&c, Some("Toji Fushiguro"), &[]));
        assert!(c.others_in(0.0, 3.0, "Toji").is_empty());
    }

    fn clip(desc: &str, tags: &[&str], title: &str) -> Clip {
        let mut c: Clip = serde_json::from_str(r#"{"file":"x.mp4","source":"","source_title":"","start":0,"duration":3.0,"motion":0}"#).unwrap();
        c.desc = desc.into();
        c.tags = tags.iter().map(|s| s.to_string()).collect();
        c.source_title = title.into();
        c
    }

    /// Casos do TODO: sobrenome de familia, particula de nome, palavra comum.
    #[test]
    fn person_needs_whole_name_parts() {
        assert!(same_person("Toji", "Toji Fushiguro"));
        assert!(same_person("Fushiguro Toji", "Toji Fushiguro"));
        assert!(!same_person("Megumi Fushiguro", "Toji Fushiguro"));
        assert!(!same_person("Zeke Yeager", "Eren Yeager"));
        assert!(!same_person("Monkey D. Garp", "Monkey D. Luffy"));
        assert!(same_person("Luffy", "Monkey D. Luffy"));
        assert!(!same_person("Que", "Que Nao"));
    }

    #[test]
    fn text_needs_unambiguous_mention() {
        let known: Vec<String> = ["Toji Fushiguro", "Megumi Fushiguro", "Eren Yeager", "Zeke Yeager"].iter().map(|s| s.to_string()).collect();
        // so o sobrenome da familia: nao prova Toji
        assert!(!text_mentions(&clip("Megumi Fushiguro invoca os cachorros", &[], ""), "Toji Fushiguro", &known));
        assert!(!text_mentions(&clip("fushiguro de costas", &[], ""), "Toji Fushiguro", &known));
        assert!(text_mentions(&clip("Toji sorri", &[], ""), "Toji Fushiguro", &known));
        assert!(text_mentions(&clip("", &["toji", "fushiguro"], ""), "Toji Fushiguro", &known));
        assert!(!text_mentions(&clip("Zeke Yeager grita", &[], ""), "Eren Yeager", &known));
        // palavra inteira: "Erenzinho" nao e "Eren"; e palavra comum nunca casa
        assert!(!text_mentions(&clip("erenzinho", &[], ""), "Eren Yeager", &known));
        assert!(!text_mentions(&clip("que cena", &[], ""), "Que", &[]));
        // titulo da fonte so sem descricao/tags
        assert!(!text_mentions(&clip("Gojo sozinho", &[], "Gojo vs Toji"), "Toji Fushiguro", &known));
        assert!(text_mentions(&clip("", &[], "Gojo vs Toji"), "Toji Fushiguro", &known));
    }

    /// Fala com quem fala identificado: vale o catalogo, nao o texto.
    #[test]
    fn speaker_decides_speech() {
        let mut c = clip("Toji e Megumi", &[], "");
        c.speech = vec![serde_json::json!({"text": "oi", "speaker": "Megumi Fushiguro"})];
        assert!(!is_character(&c, Some("Toji Fushiguro"), &[]));
        c.speech = vec![serde_json::json!({"text": "oi", "speaker": "Toji"})];
        assert!(is_character(&c, Some("Toji Fushiguro"), &[]));
        // speaker desconhecido: volta ao texto
        c.speech = vec![serde_json::json!({"text": "oi", "speaker": "desconhecido"})];
        assert!(is_character(&c, Some("Toji Fushiguro"), &[]));
    }

    /// Medida no banco real (`TKD_REAL_BANK=caminho/clips.json`, `TKD_REAL_CHAR=Nome`): quantos clipes a regra
    /// antiga (qualquer parte do nome, substring, com titulo) aceitava e a nova recusa.
    #[test]
    #[ignore]
    fn measure_real_bank() {
        let (Ok(p), Ok(name)) = (std::env::var("TKD_REAL_BANK"), std::env::var("TKD_REAL_CHAR")) else { return };
        let bank: Vec<Clip> = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let names_p = std::path::Path::new(&p).with_file_name("names.json");
        let mut known: Vec<String> = std::fs::read_to_string(names_p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        known.extend(std::env::var("TKD_REAL_KNOWN").unwrap_or_default().split('|').filter(|s| !s.is_empty()).map(String::from));
        let old = |c: &Clip| {
            let hay = format!("{} {} {}", c.desc, c.tags.join(" "), c.source_title).to_lowercase();
            name.split_whitespace().filter(|p| p.chars().count() >= 3).any(|p| hay.contains(&p.to_lowercase()))
        };
        let (mut both, mut only_old, mut only_new) = (0, vec![], 0);
        for c in &bank {
            match (old(c), is_character(c, Some(&name), &known)) {
                (true, true) => both += 1,
                (true, false) => only_old.push(format!("{} | {} | {}", c.file, c.desc.chars().take(60).collect::<String>(), c.source_title)),
                (false, true) => only_new += 1,
                _ => {}
            }
        }
        eprintln!("{name}: {} clipes; os dois aceitam {both}; so a antiga {}; so a nova {only_new}", bank.len(), only_old.len());
        for l in only_old.iter().take(15) {
            eprintln!("  saiu: {l}");
        }
    }

    #[test]
    fn source_without_burned_subs() {
        assert!(subbed_title("Gojo vs Toji Fushiguro - Full Fight | 4K | 60FPS | English Sub"));
        assert!(subbed_title("Toji's death [Eng Sub]"));
        assert!(subbed_title("Toji vs Gojo LEGENDADO"));
        assert!(!subbed_title("Gojo Vs Toji English Dub | Toji Kills Gojo (4K)"));
        assert!(!subbed_title("Toji vs Gojo 4K no subs"));
        assert!(!subbed_title("Subaru Natsuki best scenes"));
        let hit = |id: &str, t: &str| SearchHit { id: id.into(), title: t.into(), duration: Some(300.0) };
        let hits = vec![hit("aaaaaaaaaaa", "Toji scene Eng Sub"), hit("bbbbbbbbbbb", "Toji scene"), hit("ccccccccccc", "Toji scene English Dub")];
        assert_eq!(pick_hit(&hits, 600.0, &Default::default()).unwrap().id, "ccccccccccc");
    }

    #[test]
    fn crop_above_caption() {
        // 1080p sem tarja, legenda a partir de 85%: fica 0..901 (1,5% de folga)
        assert_eq!(subs_crop(1080, 0, 0.85, 1080), (900, 0));
        // com letterbox de 140 px em cima: a area util comeca em 140
        assert_eq!(subs_crop(800, 140, 0.85, 1080), (760, 140));
    }

    #[test]
    fn source_without_hd_format() {
        assert!(no_hd_format("yt-dlp falhou: ERROR: [youtube] EqNj0uOJFUs: Requested format is not available. Use --list-formats"));
        assert!(!no_hd_format("ERROR: Sign in to confirm you're not a bot ... Requested format is not available"));
    }

    #[test]
    fn ids_from_urls() {
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.tiktok.com/@x/video/1"), None);
    }
}

/// `tkd moments`: roda o detector num video (local ou do YouTube) e imprime os momentos, sem recortar.
pub async fn inspect_clip(
    file: Option<PathBuf>,
    url: Option<String>,
    channel: Option<&str>,
    profile: Option<String>,
    speech: Option<String>,
    top: usize,
) -> Result<()> {
    media::check_ffmpeg().await;
    anyhow::ensure!(file.is_some() || url.is_some(), "passe --file ou --url");
    let mut ctx = match channel {
        Some(c) => MomCtx::for_channel(&crate::config::get_channel(c)?, speech.is_some(), None),
        None => MomCtx {
            profile: if speech.is_some() { "speech".into() } else { "fight".into() },
            ..Default::default()
        },
    };
    if let Some(p) = profile {
        ctx.profile = p;
    }

    let dir = crate::config::root().join("out").join("moments");
    std::fs::create_dir_all(&dir)?;
    let mut fetched = None;
    let mut _download_guard: Option<crate::proc::RemoveFamily> = None;
    let video = match (&file, &url) {
        (Some(f), _) => f.clone(),
        (None, Some(u)) => {
            let dst = std::env::temp_dir()
                .join(format!("tkd-moments-src-{}.mp4", chrono::Utc::now().format("%H%M%S")));
            // erro no meio do download (rede caiu) deixava o mp4 e os .part no %TEMP%: so o fim do comando apagava
            _download_guard = Some(crate::proc::RemoveFamily::new(dst.clone()));
            tracing::info!("baixando {u}");
            ytdlp(&[
                u, "--no-playlist", "-f", "bv*[height<=1080]+ba/b[height<=1080]/b",
                "--merge-output-format", "mp4", "-o", &dst.to_string_lossy(), "--no-progress", "--quiet",
            ])
            .await?;
            anyhow::ensure!(dst.exists(), "download nao gerou arquivo");
            fetched = Some(dst.clone());
            dst
        }
        _ => unreachable!(),
    };
    // arquivo vazio/corrompido/1 frame passava como "0 s de video" e gravava um resultado vazio como se fosse bom
    let dur = crate::media::probe(&video).await.map(|p| p.duration).unwrap_or(0.0);
    anyhow::ensure!(
        dur >= 1.0,
        "{} nao e um video util ({dur:.2} s): arquivo vazio, corrompido ou curto demais",
        video.display()
    );
    if let Some(u) = &url {
        ctx.meta = download_meta(u, &dir.join("src.mp4")).await;
    }

    // fala (opcional): speech.py file -> json, reaproveitado pelo detector
    let mut speech_json = None;
    if let Some(l) = &speech {
        let sj = std::env::temp_dir().join("tkd-moments-speech.json");
        let out = Command::new(crate::recreate::venv_bin("python"))
            .args(["-I", "-X", "utf8", "scripts/speech.py", "file", &video.to_string_lossy(), "--lang", l])
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("TKD_FFMPEG", media::ffmpeg_bin())
            .current_dir(crate::config::root())
            .tree_output()
            .await
            .context("scripts/speech.py file")?;
        if out.status.success() {
            std::fs::write(&sj, &out.stdout)?;
            speech_json = Some(sj);
        } else {
            tracing::warn!("speech.py falhou; seguindo sem fala");
        }
    }

    let t0 = std::time::Instant::now();
    let r = find_moments(&video, &ctx, speech_json.as_deref(), top)
        .await
        .context("moments.py falhou (veja o log)")?;
    let nome = url
        .as_deref()
        .and_then(|u| u.split("v=").nth(1).map(|s| s.chars().take(11).collect::<String>()))
        .or_else(|| video.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "video".into());
    let dst = dir.join(format!("{nome}.json"));
    std::fs::write(
        &dst,
        serde_json::to_string_pretty(&serde_json::json!({
            "video": video, "url": url, "duration": r.duration, "weights": r.weights,
            "signals": r.signals, "moments": r.moments,
        }))?,
    )?;
    println!(
        "{:.0}s de video em {:.1}s | pesos {} | sinais {}",
        r.duration,
        t0.elapsed().as_secs_f64(),
        r.weights,
        r.signals
    );
    for (i, m) in r.moments.iter().enumerate() {
        let fmt = |t: f64| format!("{}:{:04.1}", (t / 60.0) as u64, t % 60.0);
        println!("{:>2}. {} - {}  nota {:+.2}  {}", i + 1, fmt(m.start), fmt(m.end), m.score, m.breakdown);
    }
    println!("-> {}", dst.display());

    for p in [ctx.meta.as_ref(), fetched.as_ref(), speech_json.as_ref()].into_iter().flatten() {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}
