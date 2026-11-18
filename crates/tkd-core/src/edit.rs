//! Video do piloto para canal de EDIT (formato "edit"): um personagem por video, footage e
//! falas reais dele (nunca TTS), estrutura copiada de uma referencia que rendeu.
//!
//! personagem (rodizio) -> banco de clipes/falas (baixa se faltar) -> referencia -> recreate
//! -> legenda/hashtags -> Spec registrada (mp4 em out/renders/<id>.mp4). Dai em diante segue
//! o mesmo caminho dos videos de texto: review (modelo local opcional + Claude) e publish.

use crate::{agent, config::Channel, footage, recreate, spec::Spec, state::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Banco minimo de um personagem antes de montar o edit (clipes de imagem, contando os que tem fala).
/// Abaixo disso o edit vira loop (18/11: 2-6 clipes deram 87-90 planos repetidos e 3 reprovacoes seguidas).
const MIN_IMAGE_CLIPS: usize = crate::recreate::MIN_DISTINCT_CLIPS;
/// Personagens tentados por edit quando o banco do primeiro nao chega ao minimo (cada um busca footage).
const MAX_CHARACTER_TRIES: usize = 2;
const MIN_SPEECH_CLIPS: usize = 3;
/// Referencias usadas nos ultimos N edits nao se repetem.
const REF_COOLDOWN: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogEntry {
    id: String,
    character: String,
    reference: String,
    at: String,
    /// pasta do recreate (analysis/<canal>/recreate/<run>): plano real do edit, que a review le
    #[serde(default, skip_serializing_if = "String::is_empty")]
    run: String,
}

fn log_path(ch: &Channel) -> PathBuf {
    ch.knowledge_dir().join("edits.json")
}

fn load_log(ch: &Channel) -> Vec<LogEntry> {
    crate::config::load_json(&log_path(ch))
}

fn save_log(ch: &Channel, l: &[LogEntry]) -> Result<()> {
    crate::config::save_json(&log_path(ch), l)
}

/// Personagem do proximo edit: o que apareceu menos (empate: o que esta ha mais tempo sem sair).
pub fn pick_character(ch: &Channel) -> Result<String> {
    Ok(characters_in_order(ch)?.into_iter().next().unwrap_or_default())
}

/// Personagens na ordem da rotacao (o primeiro e o `pick_character`).
fn characters_in_order(ch: &Channel) -> Result<Vec<String>> {
    anyhow::ensure!(
        !ch.characters.is_empty(),
        "[{}] canal de edit sem personagens: adicione em Editar canal > Personagens",
        ch.slug
    );
    let log = load_log(ch);
    let uses = |c: &str| log.iter().filter(|e| e.character.eq_ignore_ascii_case(c)).count();
    let last = |c: &str| log.iter().rposition(|e| e.character.eq_ignore_ascii_case(c)).map(|i| i as i64).unwrap_or(-1);
    let mut order = ch.characters.clone();
    order.sort_by_key(|c| (uses(c), last(c)));
    Ok(order)
}

fn bank_counts(ch: &Channel, character: &str) -> (usize, usize) {
    let bank = footage::load_bank(ch);
    // fala vale mesmo em clipe so-audio (imagem ruim): e a mesma regra do speech_digest que o agente recebe.
    // Contar so os `usable` dava 0 falas com falas no prompt -> voice "none" -> voiceClips descartados (video mudo).
    let (mut img, mut speech_line) = (0, 0);
    let known = footage::known_names(ch);
    for c in bank.iter().filter(|c| footage::is_character(c, Some(character), &known)) {
        if !c.speech.is_empty() {
            speech_line += 1;
        } else if c.usable {
            img += 1;
        }
    }
    (img, speech_line)
}

/// Garante clipes de imagem e falas reais do personagem no banco do canal.
async fn ensure_bank(ch: &Channel, store: &Store, character: &str) -> Result<(usize, usize)> {
    let (img, speech_line) = bank_counts(ch, character);
    // guarda o motivo da falha da busca: sem ele o erro final so diz "0 clipes" e esconde a causa
    let mut why = String::new();
    if img < MIN_IMAGE_CLIPS {
        tracing::info!("[{}] banco de {character}: {img} clipes de imagem, buscando mais", ch.slug);
        if let Err(e) = footage::fetch(ch, store, None, 3, 6, None, Some(character)).await {
            tracing::warn!("[{}] footage de {character}: {e:#}", ch.slug);
            why = format!("{e:#}");
        }
    }
    if speech_line < MIN_SPEECH_CLIPS {
        tracing::info!("[{}] banco de {character}: {speech_line} falas, buscando mais", ch.slug);
        let lang = if ch.edit_lang.is_empty() { "auto" } else { ch.edit_lang.as_str() };
        if let Err(e) = footage::fetch(ch, store, None, 2, 4, Some(lang), Some(character)).await {
            tracing::warn!("[{}] falas de {character}: {e:#}", ch.slug);
        }
    }
    let (img, speech_line) = bank_counts(ch, character);
    if img < 4 {
        if why.contains("YouTube bloqueou") || why.contains("orcamento do YouTube") {
            anyhow::bail!("[{}] sem cenas de {character} ({img} no banco): {why}", ch.slug);
        }
        let cause = if why.is_empty() { "a busca não achou vídeos bons (720p+)".to_string() } else { why };
        anyhow::bail!("[{}] banco de {character} fraco demais ({img} clipes de imagem): {cause}", ch.slug);
    }
    Ok((img, speech_line))
}

/// "90.9K" -> 90900
fn likes(v: &serde_json::Value) -> f64 {
    let t = v.as_str().map(|x| x.trim().to_uppercase()).unwrap_or_default();
    let (n, m) = match t.chars().last() {
        Some('K') => (&t[..t.len() - 1], 1e3),
        Some('M') => (&t[..t.len() - 1], 1e6),
        _ => (t.as_str(), 1.0),
    };
    n.replace(',', ".").parse::<f64>().map(|x| x * m).unwrap_or(0.0)
}

/// Primeiro plano parado e longo = abertura que nao e edit (gravacao de tela, meme com print).
/// Edits abrem lento as vezes, mas com movimento: das 14 refs do canal de edits so o meme do DIO (6.9 s, mov 0.004) cai aqui.
const STATIC_INTRO_S: f64 = 4.0;
const STATIC_INTRO_MOTION: f64 = 0.01;

/// Motivo para nao usar a ref como estrutura de edit, ou None se serve.
fn ref_rejection(v: &serde_json::Value) -> Option<String> {
    let a = &v["analysis"];
    if a["is_slideshow"].as_bool().unwrap_or(false) {
        return Some("slideshow".into());
    }
    let dur = a["probe"]["duration"].as_f64().unwrap_or(0.0);
    if !(6.0..=45.0).contains(&dur) {
        return Some(format!("{dur:.0}s (fora de 6-45 s)"));
    }
    let first = &a["shots"][0];
    let (d0, m0) = (first["duration"].as_f64().unwrap_or(0.0), first["motion"].as_f64().unwrap_or(1.0));
    if d0 >= STATIC_INTRO_S && m0 < STATIC_INTRO_MOTION {
        return Some(format!("abre com {d0:.1}s parados (gravacao de tela/meme, nao edit)"));
    }
    None
}

/// Peso extra para ref no formato vertical: o edit sai no formato do canal de qualquer jeito, mas a posicao de
/// texto/legenda da ref so se copia bem do mesmo formato.
fn aspect_weight(v: &serde_json::Value) -> f64 {
    let (w, h) = (v["analysis"]["probe"]["width"].as_f64().unwrap_or(0.0), v["analysis"]["probe"]["height"].as_f64().unwrap_or(0.0));
    if w > 0.0 && h / w >= 1.6 { 1.3 } else { 1.0 }
}

/// Referencia de estrutura: a que mais rendeu entre as baixadas, sem repetir as recentes.
/// Com fala de personagem no audio ganha preferencia (o edit usa falas reais); vertical (9:16) tambem.
pub fn pick_reference(ch: &Channel) -> Result<String> {
    let recent: Vec<String> = load_log(ch).iter().rev().take(REF_COOLDOWN).map(|e| e.reference.clone()).collect();
    let mut cands: Vec<(f64, String)> = Vec::new();
    for e in std::fs::read_dir(ch.ref_meta_dir()).context("sem referencias: estude o canal antes")?.flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(e.path()).unwrap_or_default()) else { continue };
        let id = v["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() || !ch.refs_dir().join(format!("{id}.mp4")).exists() {
            continue;
        }
        if let Some(why) = ref_rejection(&v) {
            tracing::debug!("[{}] ref {id} fora: {why}", ch.slug);
            continue;
        }
        let speech_line = v["sound"]["speech"]["seconds"].as_f64().unwrap_or(0.0) >= 1.0;
        let w = likes(&v["stats"]["likes"]) * if speech_line { 1.5 } else { 1.0 } * aspect_weight(&v);
        cands.push((w, id));
    }
    anyhow::ensure!(
        !cands.is_empty(),
        "[{}] nenhuma referencia baixada serve de estrutura (6-45 s, sem abertura parada)",
        ch.slug
    );
    cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let pick = cands.iter().find(|(_, id)| !recent.contains(id)).unwrap_or(&cands[0]);
    Ok(pick.1.clone())
}

#[derive(Debug, Deserialize)]
struct CaptionOut {
    caption: String,
    #[serde(default)]
    hashtags: Vec<String>,
}

/// Falas/textos que o edit usa, lidos do plano do recreate (analysis/<canal>/recreate/<run>/agent.json).
fn spoken_lines(ch: &Channel, out: &std::path::Path) -> Vec<String> {
    let run = out.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let p = crate::config::root().join("analysis").join(&ch.slug).join("recreate").join(run).join("agent.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_default()).unwrap_or_default();
    edit_lines(&v)
}

/// Textos falados/escritos em qualquer lugar do agent.json do recreate (sem repetir).
fn collect_lines(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                match (k.as_str(), x.as_str()) {
                    ("text" | "line" | "say" | "words_text", Some(t)) if t.chars().count() > 2 && !out.iter().any(|o| o == t) => {
                        out.push(t.to_string())
                    }
                    _ => collect_lines(x, out),
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_lines(x, out)),
        _ => {}
    }
}

async fn caption(ch: &Channel, character: &str, lines: &[String]) -> Result<CaptionOut> {
    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("persona", ch.persona.clone());
    vars.insert("niche", ch.niche.clone());
    vars.insert("brand", crate::brand::brand_of(ch));
    vars.insert("character", character.to_string());
    vars.insert("lines", if lines.is_empty() { "(sem fala)".into() } else { lines.join(" / ") });
    vars.insert("hashtags", if ch.hashtags.is_empty() { "(nenhuma)".into() } else { ch.hashtags.join(" ") });
    let prompt = agent::render_template("edit_caption.md", &vars)?;
    let mut out: CaptionOut = agent::ask_json(ch, "caption", &prompt, &[], 180, 2).await?;
    let names = crate::brand::reference_names(ch);
    out.caption = crate::brand::scrub_with(&out.caption, &names, &crate::brand::brand_of(ch)).0;
    out.hashtags.retain(|t| !crate::brand::tag_mentions_reference(t, &names));
    Ok(out)
}

/// Spec minima de um edit pronto: o mp4 ja vem renderizado pelo recreate; a spec serve pra
/// review/publish (legenda, hashtags) e marca o video como footage (avaliado pelo modelo local, se houver).
fn edit_spec(ch: &Channel, id: &str, character: &str, dur: f64, cap: &CaptionOut, reference: &str) -> Result<Spec> {
    let preset: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ch.preset()).unwrap_or_default()).unwrap_or_default();
    let pal = &preset["palette"];
    let v = serde_json::json!({
        "id": id,
        "channel": ch.slug,
        "title": character,
        "durationS": dur,
        "aspect": preset["aspect"].as_str().unwrap_or("9:16"),
        "fps": preset["fps"].as_f64().unwrap_or(30.0),
        "palette": {
            "bg": pal["bg"].as_str().unwrap_or("#000000"),
            "fg": pal["fg"].as_str().unwrap_or("#ffffff"),
            "accent": pal["accent"].as_str().unwrap_or("#ff2e63"),
        },
        "typography": { "family": "Anton", "weight": 400, "case": "upper", "sizeVh": 6.0 },
        "treatment": { "grain": 0.0, "vignette": 0.0, "zoom": "none" },
        "scenes": [{
            "startS": 0.0, "durationS": dur, "kind": "media", "text": "",
            "media": format!("edit/{reference}.mp4"),
        }],
        "caption": cap.caption,
        "hashtags": cap.hashtags,
    });
    serde_json::from_value(v).context("montando a spec do edit")
}

/// Monta um edit completo e registra como video do canal. Devolve a spec (mp4 em out/renders).
pub async fn make_edit(ch: &Channel, store: &Store) -> Result<Spec> {
    let span = crate::events::step(&ch.slug, "ideate");
    span.action("Escolhendo personagem e referência", "claude");
    let r = make_inner(ch, store, &span).await;
    span.finish(r, |s| s.title.clone())
}

async fn make_inner(ch: &Channel, store: &Store, span: &crate::events::Span) -> Result<Spec> {
    // banco pequeno demais vira edit em loop: passa para o proximo da rotacao (que tambem busca footage)
    let mut skipped: Vec<String> = vec![];
    let mut chosen = None;
    for character in characters_in_order(ch)?.into_iter().take(MAX_CHARACTER_TRIES) {
        span.action(&format!("Juntando clipes e falas de {character}"), "python");
        let why = match ensure_bank(ch, store, &character).await {
            Ok((img, speech_line)) => {
                let image = crate::recreate::character_clips(ch, &character);
                if image >= MIN_IMAGE_CLIPS {
                    chosen = Some((character, img, speech_line));
                    break;
                }
                format!("{image} clipes de imagem (minimo {MIN_IMAGE_CLIPS})")
            }
            Err(e) => format!("{e:#}"),
        };
        tracing::warn!("[{}] {character} fica para depois: {why}", ch.slug);
        skipped.push(format!("{character}: {why}"));
    }
    let Some((character, img, speech_line)) = chosen else {
        anyhow::bail!("[{}] nenhum personagem com banco suficiente para um edit: {}", ch.slug, skipped.join("; "));
    };
    let reference = pick_reference(ch)?;
    tracing::info!("[{}] edit de {character} ({img} clipes, {speech_line} falas) na estrutura da ref {reference}", ch.slug);

    span.action(&format!("Montando o edit de {character}"), "claude");
    let voice = if speech_line > 0 { "clip" } else { "none" };
    let out = recreate::recreate(
        ch,
        recreate::Opts {
            ref_: reference.clone(),
            script: String::new(),
            end_s: None,
            voice_id: std::env::var("TKD_VOICE_ID").unwrap_or_default(),
            plan: None,
            compare: false,
            voice: voice.into(),
            character: Some(character.clone()),
            motion: None,
            lang: if ch.edit_subs.is_empty() { "en".into() } else { ch.edit_subs.clone() },
        },
    )
    .await?;
    let probe = crate::media::probe(&out).await?;

    span.action("Escrevendo a legenda", "claude");
    let cap = caption(ch, &character, &spoken_lines(ch, &out)).await?;

    let id = format!("{}-{}", ch.slug, chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let spec = edit_spec(ch, &id, &character, probe.duration, &cap, &reference)?;
    let mp4 = crate::render::mp4_path(&id);
    std::fs::create_dir_all(mp4.parent().unwrap_or(std::path::Path::new(".")))?;
    crate::media::link_or_copy(&out, &mp4).context("copiando o edit pra out/renders")?;
    std::fs::create_dir_all(crate::ideate::specs_dir())?;
    std::fs::write(crate::ideate::spec_path(&id), serde_json::to_string_pretty(&spec)?)?;
    store.record_video(&id, &ch.slug, &serde_json::to_string(&spec)?, &mp4.to_string_lossy())?;
    store.log(&ch.slug, "ideate", "ok", &format!("edit de {character} (ref {reference})"))?;

    let mut log = load_log(ch);
    let run = out.file_stem().unwrap_or_default().to_string_lossy().to_string();
    log.push(LogEntry { id: id.clone(), character, reference, at: crate::state::now(), run });
    save_log(ch, &log)?;
    Ok(spec)
}

/// O que o edit `id` realmente e, para a review do Claude: a spec de um edit e so um invólucro (1 cena com o mp4
/// pronto) e o Claude chegou a responder "o video nao foi editado" (canal de edits, 17/11, nota 2.8).
/// Le o plano do recreate (cortes, efeitos, trilha, falas, legendas) e o QA. None = edit sem registro do recreate.
pub fn review_context(ch: &Channel, id: &str) -> Option<String> {
    let e = load_log(ch).into_iter().find(|e| e.id == id)?;
    let root = crate::config::root();
    let run = if e.run.is_empty() {
        // edits de antes do campo `run`: o recreate mais recente da mesma ref ate o horario do edit
        let dir = root.join("analysis").join(&ch.slug).join("recreate");
        let mut runs: Vec<String> = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .map(|d| d.file_name().to_string_lossy().to_string())
            .filter(|r| r.starts_with(&format!("{}-", e.reference)))
            .collect();
        runs.sort();
        runs.pop()?
    } else {
        e.run.clone()
    };
    let rdir = root.join("analysis").join(&ch.slug).join("recreate").join(&run);
    let read = |f: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(rdir.join(f)).unwrap_or_default()).unwrap_or_default()
    };
    let plan = read("props.json");
    let plan = if plan.is_null() { read("plan.json") } else { plan };
    if plan.is_null() {
        return None;
    }
    Some(describe_plan(&plan, &read("qa.json"), &e.character, &e.reference, &edit_lines(&read("agent.json")), ch, &rdir))
}

fn edit_lines(agent: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_lines(agent, &mut out);
    // legenda "maquina de escrever" vem em passos ("the", "the s", "the sorc"...): fica so o texto completo
    let all = out.clone();
    out.retain(|l| !all.iter().any(|o| o.len() > l.len() && o.starts_with(l.as_str())));
    out.truncate(8);
    out
}

fn describe_plan(
    plan: &serde_json::Value,
    qa: &serde_json::Value,
    character: &str,
    reference: &str,
    lines: &[String],
    ch: &Channel,
    rdir: &std::path::Path,
) -> String {
    let empty = Vec::new();
    let shots = plan["shots"].as_array().unwrap_or(&empty);
    let fps = plan["fps"].as_f64().unwrap_or(30.0).max(1.0);
    let total = plan["durationInFrames"].as_f64().unwrap_or(0.0) / fps;
    let lens: Vec<f64> = shots
        .iter()
        .map(|s| (s["to"].as_f64().unwrap_or(0.0) - s["from"].as_f64().unwrap_or(0.0)) / fps)
        .collect();
    let avg = if lens.is_empty() { 0.0 } else { lens.iter().sum::<f64>() / lens.len() as f64 };
    let srcs: std::collections::BTreeSet<&str> = shots.iter().filter_map(|s| s["src"].as_str()).collect();
    let mut fx: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for s in shots {
        for k in ["fx", "transitionIn"] {
            let name = match &s[k] {
                serde_json::Value::String(x) => x.clone(),
                serde_json::Value::Object(o) => o.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                _ => String::new(),
            };
            if !name.is_empty() {
                *fx.entry(name).or_default() += 1;
            }
        }
    }
    for v in plan["frameFx"].as_object().into_iter().flat_map(|o| o.values()) {
        if let Some(x) = v.as_str() {
            *fx.entry(format!("flash {x}")).or_default() += 1;
        }
    }
    let music = plan["audio"]["music"].as_str().filter(|m| !m.is_empty());
    let music = match music {
        Some(m) => {
            let t = crate::music::beats_of(ch, m);
            format!(
                "{m}{}",
                t.map(|t| format!(" (\"{}\", {:.0} bpm{})", t.title, t.bpm, if t.ref_id.is_some() { ", musica da propria ref" } else { "" }))
                    .unwrap_or_default()
            )
        }
        None => "nenhuma".into(),
    };
    let voice = plan["voiceClips"].as_array().map(|v| v.len()).unwrap_or(0);
    let qa_line = if qa.is_null() {
        "QA nao rodou".to_string()
    } else {
        let issues: Vec<String> = qa["issues"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .map(|i| format!("[{}] {}", i["level"].as_str().unwrap_or("?"), i["what"].as_str().unwrap_or("")))
            .collect();
        format!(
            "QA {} (nota {}){}",
            if qa["pass"].as_bool().unwrap_or(false) { "passou" } else { "reprovou" },
            qa["score"],
            if issues.is_empty() { String::new() } else { format!(": {}", issues.join("; ")) }
        )
    };
    let adir = crate::config::root().join("analysis").join(&ch.slug).join(reference);
    let sheet = Some(crate::recreate::sheet_path(&adir)).filter(|p| p.exists()).unwrap_or_else(|| adir.join("sheet.jpg"));
    let mut s = format!(
        "Edit de footage MONTADO pelo recreate ({}): {total:.1}s, {} planos (média {avg:.2}s por plano), \
         {} clipes diferentes do personagem `{character}`.\n",
        rdir.display(),
        shots.len(),
        srcs.len()
    );
    s.push_str(&format!("- Estrutura copiada da referência {reference}"));
    if sheet.exists() {
        s.push_str(&format!(" (contact sheet da ref, abra para comparar: {})", sheet.display()));
    }
    s.push('\n');
    if !fx.is_empty() {
        s.push_str(&format!(
            "- Efeitos/transições: {}\n",
            fx.iter().map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", ")
        ));
    }
    s.push_str(&format!("- Trilha: {music}\n- Falas reais do personagem no edit: {voice}\n"));
    if !lines.is_empty() {
        s.push_str(&format!("- Texto/falas: {}\n", lines.join(" / ")));
    }
    s.push_str(&format!("- {qa_line}\n"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_and_footage_spec() {
        let ch = Channel { slug: "x".into(), ..Default::default() };
        let cap = CaptionOut { caption: "Toji não perdoa".into(), hashtags: vec!["#toji".into()] };
        let s = edit_spec(&ch, "x-1", "Toji Fushiguro", 18.0, &cap, "123").unwrap();
        assert!(crate::local_eval::in_domain(&s, &crate::local_eval::EvaluatorCfg::default()));
        assert_eq!(s.caption, "Toji não perdoa");
    }

    #[test]
    fn ref_with_static_intro_is_skipped() {
        let r = |d0: f64, m0: f64, w: f64, h: f64| {
            serde_json::json!({"analysis": {"probe": {"duration": 25.0, "width": w, "height": h},
                                            "shots": [{"duration": d0, "motion": m0}]}})
        };
        // meme do DIO do canal de edits (17/11)
        assert!(ref_rejection(&r(6.9, 0.0043, 1080.0, 1078.0)).is_some());
        // abertura lenta com movimento (ragna, 14.3 s) continua valendo
        assert!(ref_rejection(&r(14.3, 0.0478, 1064.0, 1080.0)).is_none());
        assert!(ref_rejection(&r(2.0, 0.001, 1080.0, 1920.0)).is_none());
        assert_eq!(aspect_weight(&r(2.0, 0.1, 1080.0, 1920.0)), 1.3);
        assert_eq!(aspect_weight(&r(2.0, 0.1, 1080.0, 1080.0)), 1.0);
    }

    #[test]
    fn legenda_em_passos_vira_uma_linha() {
        let a = serde_json::json!({"captions": [{"text": "the"}, {"text": "the sorc"}, {"text": "the sorcerer killer"}],
                                   "title": {"text": "ANIME"}});
        assert_eq!(edit_lines(&a), vec!["the sorcerer killer".to_string(), "ANIME".to_string()]);
    }

    #[test]
    fn character_rotation() {
        let dir = std::env::temp_dir().join(format!("tkd-edit-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("knowledge")).unwrap();
        let ch = Channel { slug: "x".into(), dir: dir.clone(), characters: vec!["A".into(), "B".into()], ..Default::default() };
        assert_eq!(pick_character(&ch).unwrap(), "A");
        let log = vec![LogEntry { id: "1".into(), character: "A".into(), reference: "r".into(), at: "t".into(), run: String::new() }];
        save_log(&ch, &log).unwrap();
        assert_eq!(pick_character(&ch).unwrap(), "B");
        assert_eq!(characters_in_order(&ch).unwrap(), vec!["B", "A"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}

