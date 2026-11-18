use serde::{Deserialize, Serialize};

/// Espelho de video/src/schema.ts — o que o agente produz e o Remotion consome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    pub bg: String,
    pub fg: String,
    pub accent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watermark {
    #[serde(default)]
    pub text: String,
    #[serde(rename = "sizeVh", default = "two_two")]
    pub size_vh: f64,
    #[serde(default = "half")]
    pub opacity: f64,
    #[serde(default = "gray")]
    pub color: String,
}

fn two_two() -> f64 { 2.2 }
fn half() -> f64 { 0.55 }
fn gray() -> String { "#bfbfbf".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Typography {
    pub family: String,
    pub weight: u32,
    #[serde(rename = "case")]
    pub case_: String,
    #[serde(rename = "sizeVh")]
    pub size_vh: f64,
    #[serde(default = "normal_style")]
    pub style: String,
}

fn normal_style() -> String { "normal".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Treatment {
    pub grain: f64,
    pub vignette: f64,
    pub zoom: String,
    #[serde(default)]
    pub glitch: bool,
    #[serde(rename = "blurBg", default)]
    pub blur_bg: f64,
    #[serde(default = "four_tenths")]
    pub crush: f64,
    #[serde(default = "three_tenths")]
    pub bloom: f64,
}

fn four_tenths() -> f64 { 0.4 }
fn three_tenths() -> f64 { 0.3 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSpec {
    #[serde(rename = "startS")]
    pub start_s: f64,
    #[serde(rename = "durationS")]
    pub duration_s: f64,
    #[serde(default = "text_kind")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sub: String,
    #[serde(rename = "sizeVh", skip_serializing_if = "Option::is_none")]
    pub size_vh: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// segundo de entrada dentro do clipe (corta o comeco)
    #[serde(rename = "mediaStart", skip_serializing_if = "Option::is_none")]
    pub media_start: Option<f64>,
    #[serde(default = "none_effect")]
    pub effect: String,
    /// cartela gigante: o texto ocupa a largura toda
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<bool>,
    /// volume do audio do proprio clipe (0-1): fala do personagem
    #[serde(default = "zero")]
    pub volume: f64,
    /// marca interna: pedaco que sobrou de uma cena cortada
    #[serde(skip)]
    pub continuation: bool,
}

impl SceneSpec {
    fn empty_spec() -> Self {
        Self {
            start_s: 0.0,
            duration_s: 0.0,
            kind: "text".into(),
            text: String::new(),
            sub: String::new(),
            size_vh: None,
            media: None,
            media_start: None,
            effect: "none".into(),
            fill: None,
            volume: 0.0,
            continuation: false,
        }
    }
}

fn text_kind() -> String { "text".into() }
fn none_effect() -> String { "none".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub id: String,
    pub channel: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "durationS")]
    pub duration_s: f64,
    /// "9:16" | "1:1" | "4:5" | "4:3" — varios canais de edit usam 1:1; canal de frases costuma usar 4:3.
    #[serde(default = "nine_sixteen")]
    pub aspect: String,
    #[serde(default = "thirty")]
    pub fps: f64,
    pub palette: Palette,
    pub typography: Typography,
    pub treatment: Treatment,
    pub scenes: Vec<SceneSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// segundo de entrada da trilha (normalmente o drop)
    #[serde(rename = "audioStart", skip_serializing_if = "Option::is_none")]
    pub audio_start: Option<f64>,
    /// volume da trilha (0-1)
    #[serde(rename = "audioVolume", default = "eight_tenths")]
    pub audio_volume: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Watermark>,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
}

fn thirty() -> f64 { 30.0 }
fn eight_tenths() -> f64 { 0.8 }
fn zero() -> f64 { 0.0 }
fn nine_sixteen() -> String { "9:16".into() }

const FAMILIES: [&str; 5] = ["Inter", "Anton", "BebasNeue", "Poppins", "PlayfairDisplay"];
const ZOOMS: [&str; 4] = ["none", "slow-in", "slow-out", "punch"];
const KINDS: [&str; 4] = ["text", "quote", "media", "title"];
const EFFECTS: [&str; 7] = ["none", "shake", "flash", "slide", "typewriter", "words", "invert"];

impl Spec {
    /// Conserta o que da para consertar e rejeita o que quebraria o render.
    /// `clip_len` da a duracao de cada clipe do banco: e o que impede
    /// uma cena de 6s rodar sobre um clipe de 1.8s e congelar no ultimo frame.
    pub fn validate_and_fix(
        &mut self,
        assets: &[String],
        clip_len: &std::collections::HashMap<String, f64>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.scenes.is_empty(), "spec sem cenas");
        anyhow::ensure!(
            (5.0..=40.0).contains(&self.duration_s),
            "duracao {:.1}s fora de 5-40s",
            self.duration_s
        );
        if !FAMILIES.contains(&self.typography.family.as_str()) {
            self.typography.family = "Inter".into();
        }
        if self.typography.case_ != "upper" && self.typography.case_ != "normal" {
            self.typography.case_ = "upper".into();
        }
        if !ZOOMS.contains(&self.treatment.zoom.as_str()) {
            self.treatment.zoom = "slow-in".into();
        }
        self.treatment.grain = self.treatment.grain.clamp(0.0, 1.0);
        self.treatment.vignette = self.treatment.vignette.clamp(0.0, 1.0);
        self.treatment.blur_bg = self.treatment.blur_bg.clamp(0.0, 40.0);
        self.audio_volume = self.audio_volume.clamp(0.0, 1.0);
        self.treatment.crush = self.treatment.crush.clamp(0.0, 1.0);
        self.treatment.bloom = self.treatment.bloom.clamp(0.0, 1.0);
        if self.typography.style != "italic" {
            self.typography.style = "normal".into();
        }
        self.fps = 30.0;
        if !["9:16", "1:1", "4:5", "4:3"].contains(&self.aspect.as_str()) {
            self.aspect = "9:16".into();
        }

        // cenas encadeadas sem buraco; midia inexistente vira fundo gerado
        let mut cursor = 0.0;
        let mut broken: Vec<SceneSpec> = Vec::new();
        for s in self.scenes.iter_mut() {
            if !KINDS.contains(&s.kind.as_str()) {
                s.kind = "text".into();
            }
            if !EFFECTS.contains(&s.effect.as_str()) {
                s.effect = "none".into();
            }
            anyhow::ensure!(s.duration_s > 0.2, "cena com duracao {:.2}s", s.duration_s);
            if let Some(v) = s.size_vh {
                s.size_vh = Some(v.clamp(1.0, 45.0));
            }
            if let Some(m) = &s.media {
                if !assets.iter().any(|a| a == m) {
                    tracing::warn!("asset inexistente removido da cena: {m}");
                    s.media = None;
                }
            }

            // o clipe precisa ter imagem em movimento pela cena inteira
            if let (Some(m), Some(total)) = (s.media.clone(), s.media.as_ref().and_then(|m| clip_len.get(m).copied())) {
                let start_at = s.media_start.unwrap_or(0.0);
                // entrada depois do fim do clipe = congelado desde o frame 0
                let start_at = if start_at >= total - 0.3 {
                    tracing::warn!("mediaStart {start_at:.1}s passa do clipe {m} ({total:.1}s) — zerado");
                    s.media_start = Some(0.0);
                    0.0
                } else {
                    start_at
                };
                let leftover = total - start_at;
                if s.duration_s > leftover + 0.05 {
                    tracing::warn!(
                        "cena de {:.1}s sobre {:.1}s de clipe — cortada para nao congelar",
                        s.duration_s,
                        leftover
                    );
                    let remainder = s.duration_s - leftover;
                    s.duration_s = leftover;
                    // o tempo que sobrou vira continuacao, para outro clipe assumir
                    broken.push(SceneSpec {
                        start_s: 0.0,
                        duration_s: remainder,
                        kind: s.kind.clone(),
                        text: s.text.clone(),
                        sub: String::new(),
                        size_vh: s.size_vh,
                        media: None,
                        media_start: None,
                        effect: "none".into(),
                        fill: s.fill,
                        volume: 0.0,
                        continuation: true,
                    });
                } else {
                    broken.push(SceneSpec::empty_spec());
                }
            } else {
                broken.push(SceneSpec::empty_spec());
            }
            s.start_s = cursor;
            cursor += s.duration_s;
        }

        // reinsere as continuacoes logo apos a cena de origem
        let mut finals: Vec<SceneSpec> = Vec::new();
        for (s, next_scene) in self.scenes.drain(..).zip(broken.into_iter()) {
            finals.push(s);
            if next_scene.continuation && next_scene.duration_s > 0.2 {
                finals.push(next_scene);
            }
        }
        self.scenes = finals;

        // recalcula a linha do tempo ja com as continuacoes
        let mut cursor = 0.0;
        for s in self.scenes.iter_mut() {
            s.start_s = cursor;
            cursor += s.duration_s;
        }
        self.duration_s = cursor;
        Ok(())
    }
}
