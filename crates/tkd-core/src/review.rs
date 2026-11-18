use crate::{agent, config::Channel, media, render, state::Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Verdict {
    pub score: f64,
    #[serde(default)]
    pub keep: bool,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub best_part: String,
}

pub const MIN_SCORE: f64 = 7.0;

/// Avalia o mp4 e, se passar, promove para channels/<slug>/library/<id>/.
///
/// Edit (footage) com `[evaluator]` no tkd.toml: o modelo local avalia e decide sozinho quando a nota e
/// clara; o Claude so entra na faixa duvidosa, com defeito grave medido, na auditoria ou se o modelo
/// falhar — e recebe a avaliacao do modelo pra confirmar/corrigir. Sem avaliador ou video de texto: so o Claude.
pub async fn review(ch: &Channel, store: &Store, id: &str) -> Result<Verdict> {
    let span = crate::events::step(&ch.slug, "review");
    let r = review_inner(ch, store, id, &span).await;
    span.finish(r, |v| format!("nota {}", v.score))
}

/// Quem decide a partir da nota local (None = chamar o Claude) e por que.
fn local_decision(
    ev: &crate::local_eval::LocalEval,
    id: &str,
    cfg: &crate::local_eval::EvaluatorCfg,
) -> (Option<bool>, &'static str) {
    let s = ev.score();
    if s <= cfg.doubt_low {
        return (Some(false), "nota baixa clara");
    }
    if s < cfg.doubt_high {
        return (None, "nota na faixa duvidosa");
    }
    if ev.rules_grave() {
        return (None, "nota alta mas as regras mediram defeito grave");
    }
    if crate::local_eval::audit_pick(id, cfg.audit_every) {
        return (None, "auditoria das aprovações do modelo");
    }
    (Some(true), "nota alta clara")
}

fn verdict_from_local(ev: &crate::local_eval::LocalEval, keep: bool) -> Verdict {
    Verdict {
        score: ev.score(),
        keep,
        problems: ev.all_problems().iter().map(|p| p.line()).collect(),
        best_part: ev.strengths.first().map(|p| p.what_text.clone()).unwrap_or_else(|| ev.summary.clone()),
    }
}

async fn review_inner(ch: &Channel, store: &Store, id: &str, span: &crate::events::Span) -> Result<Verdict> {
    let mp4 = render::mp4_path(id);
    anyhow::ensure!(mp4.exists(), "mp4 de {id} nao existe — rode `tkd render` antes");
    let spec = render::load_spec(id)?;

    // 1. avaliador local opcional (so em edit; `[evaluator]` no tkd.toml)
    let eval_cfg = crate::local_eval::config();
    let mut local: Option<crate::local_eval::LocalEval> = None;
    let mut why_claude = "fora do que o modelo local conhece (só avalia canal de edit)";
    // corte de live: o Claude ja avaliou na escolha do corte (sem modelo local: a v4 nao conhece live)
    if ch.is_clips() {
        let verdict = crate::clips::stored_verdict(id)
            .ok_or_else(|| anyhow::anyhow!("[{}] {id}: corte sem avaliação guardada", ch.slug))?;
        let keep = verdict.keep && verdict.score >= MIN_SCORE;
        store.record_review(&ch.slug, id, None, Some(verdict.score), verdict.score, keep, "claude-clips", "corte de live")?;
        return finish(ch, store, id, &spec, &mp4, verdict);
    }
    // so canal de EDIT (nunca por adivinhacao: corte de live tambem e 100% footage)
    if !eval_cfg.enabled() {
        why_claude = "sem avaliador local configurado";
    } else if ch.is_edit() && crate::local_eval::in_domain(&spec, &eval_cfg) {
        match crate::local_eval::available(&eval_cfg) {
            Err(e) => {
                tracing::warn!("[{}] avaliador local indisponivel ({e}): review so com o Claude", ch.slug);
                why_claude = "modelo local indisponível";
            }
            Ok(_) => {
                span.action(&format!("Avaliando com o modelo local ({})", eval_cfg.name), "local");
                let caption = crate::hashtags::Post::new(&spec.caption, &spec.hashtags, &ch.hashtags, "tiktok").caption();
                match crate::local_eval::evaluate(&eval_cfg, &mp4, &caption).await {
                    Ok(ev) => {
                        tracing::info!("[{}] {id}: {} nota {:.1} em {:.0}s, regras {:?}", ch.slug, ev.model, ev.score(), ev.seconds, ev.rules);
                        let dir = crate::config::root().join("out").join("reviews");
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join(format!("{id}.local.json")), serde_json::to_string_pretty(&ev).unwrap_or_default());
                        local = Some(ev);
                    }
                    Err(e) => {
                        tracing::warn!("[{}] {id}: modelo local falhou ({e:#}); review com o Claude", ch.slug);
                        why_claude = "modelo local falhou";
                    }
                }
            }
        }
    }

    // 2. o avaliador decide sozinho quando o caso e claro
    if let Some(ev) = &local {
        let (decided, reason) = local_decision(ev, id, &eval_cfg);
        if let Some(keep) = decided {
            let verdict = verdict_from_local(ev, keep);
            store.record_review(&ch.slug, id, Some(ev.score()), None, verdict.score, keep, "local", reason)?;
            return finish(ch, store, id, &spec, &mp4, verdict);
        }
        why_claude = reason;
    }

    // 3. Claude (com a avaliacao do modelo local quando houver)
    span.action(
        if local.is_some() { "Claude conferindo a nota do modelo local" } else { "Avaliando o vídeo" },
        "claude",
    );

    let probe = media::probe(&mp4).await?;
    // edit: a spec e so o invólucro do mp4 pronto; o Claude recebe o plano real do recreate e mais frames
    // (edit corta a cada ~0.5-2 s, 6 frames nao mostram o ritmo)
    let edit_ctx = if ch.is_edit() { crate::edit::review_context(ch, id) } else { None };
    let frames_dir = crate::config::root().join("out").join("frames").join(id);
    // quadros so servem para esta chamada do Claude: nao acumular em out/frames
    let _frames_cleanup = crate::proc::RemoveOnDrop(frames_dir.clone());
    let frames = media::frames(&mp4, &frames_dir, if edit_ctx.is_some() { 12 } else { 6 }).await?;
    let frame_list = frames
        .iter()
        .map(|p| format!("- {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("style_card", std::fs::read_to_string(ch.style_card())?);
    vars.insert("spec", serde_json::to_string_pretty(&spec)?);
    vars.insert("probe", serde_json::to_string_pretty(&probe)?);
    vars.insert("frames", frame_list);
    vars.insert("notes", ch.notes_text());
    // sem trilha disponivel, cobrar audio so geraria reprovacao em serie
    let has_audio_assets = crate::ideate::assets(ch)
        .iter()
        .any(|a| a.ends_with(".mp3") || a.ends_with(".wav") || a.ends_with(".m4a"));
    vars.insert(
        "audio_note",
        if has_audio_assets {
            "o canal tem trilhas disponiveis — penalize se o video saiu mudo.".to_string()
        } else {
            "o canal ainda nao tem trilha nenhuma em video/public; IGNORE a ausencia de audio nesta avaliacao.".to_string()
        },
    );

    vars.insert(
        "local_eval",
        match &local {
            Some(ev) => format!(
                "Motivo de você estar sendo chamado: {why_claude}.\n{}\nConfirme ou corrija. Se a sua nota divergir \
                 mais de 1 ponto da do modelo, diga por quê no primeiro item de `problems`.",
                ev.brief()
            ),
            None => format!("(sem avaliação local: {why_claude})"),
        },
    );
    let template = match &edit_ctx {
        Some(ctx) => {
            vars.insert("edit", ctx.clone());
            // edit sempre sai com trilha e/ou fala real (o recreate barra video mudo antes do render)
            vars.insert("audio_note", "edit tem trilha/falas no plano acima; video mudo ou fala inaudivel = reprovar.".into());
            "review_edit.md"
        }
        None => "review.md",
    };
    let prompt = agent::render_template(template, &vars)?;
    let verdict: Verdict = agent::ask_json(ch, "review", &prompt, &["Read"], 420, 2).await?;
    let keep = verdict.keep && verdict.score >= MIN_SCORE;
    store.record_review(
        &ch.slug,
        id,
        local.as_ref().map(|e| e.score()),
        Some(verdict.score),
        verdict.score,
        keep,
        if local.is_some() { "claude+local" } else { "claude" },
        why_claude,
    )?;
    finish(ch, store, id, &spec, &mp4, verdict)
}

/// Grava a nota e, se aprovado, promove pra biblioteca.
fn finish(ch: &Channel, store: &Store, id: &str, spec: &crate::spec::Spec, mp4: &std::path::Path, verdict: Verdict) -> Result<Verdict> {
    let keep = verdict.keep && verdict.score >= MIN_SCORE;
    store.score_video(id, verdict.score, keep)?;

    if keep {
        let dir = ch.library_dir().join(id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("spec.json"), serde_json::to_string_pretty(spec)?)?;
        std::fs::write(
            dir.join("score.json"),
            serde_json::to_string_pretty(&verdict)?,
        )?;
        crate::media::link_or_copy(mp4, &dir.join("out.mp4"))?;
        store.log(&ch.slug, "review", "aprovado", &format!("{id} nota {}", verdict.score))?;
        tracing::info!("[{}] {id} APROVADO ({}) -> biblioteca", ch.slug, verdict.score);
    } else {
        store.log(
            &ch.slug,
            "review",
            "reprovado",
            &format!("{id} nota {} — {}", verdict.score, verdict.problems.join("; ")),
        )?;
        tracing::warn!(
            "[{}] {id} reprovado ({}): {}",
            ch.slug,
            verdict.score,
            verdict.problems.join("; ")
        );
    }
    Ok(verdict)
}

/// Esta na biblioteca (ou seja, foi aprovado)?
pub fn approved(ch: &Channel, id: &str) -> bool {
    ch.library_dir().join(id).join("out.mp4").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_eval::{EvaluatorCfg, LocalEval, Problem};

    fn ev(score: f64, severe: bool) -> LocalEval {
        LocalEval {
            overall: Some(score),
            rule_problems: if severe {
                vec![Problem { ptype: "audio_estourado".into(), severity: "alta".into(), ..Default::default() }]
            } else {
                vec![]
            },
            ..Default::default()
        }
    }

    #[test]
    fn decides_only_when_clear() {
        // id fora da auditoria
        let c = EvaluatorCfg::default();
        let id = (0..100).map(|i| format!("v{i}")).find(|i| !crate::local_eval::audit_pick(i, c.audit_every)).unwrap();
        assert_eq!(local_decision(&ev(6.0, false), &id, &c).0, Some(false));
        assert_eq!(local_decision(&ev(7.0, false), &id, &c).0, None);
        assert_eq!(local_decision(&ev(8.2, false), &id, &c).0, Some(true));
        assert_eq!(local_decision(&ev(8.2, true), &id, &c).0, None);
        let aud = (0..100).map(|i| format!("v{i}")).find(|i| crate::local_eval::audit_pick(i, c.audit_every)).unwrap();
        assert_eq!(local_decision(&ev(8.2, false), &aud, &c).0, None);
        // reprovacao clara nao passa por auditoria
        assert_eq!(local_decision(&ev(5.5, false), &aud, &c).0, Some(false));
        // faixa propria do usuario
        let strict = EvaluatorCfg { doubt_low: 7.0, doubt_high: 9.0, audit_every: 0, ..c };
        assert_eq!(local_decision(&ev(6.9, false), &aud, &strict).0, Some(false));
        assert_eq!(local_decision(&ev(8.2, false), &aud, &strict).0, None);
        assert_eq!(local_decision(&ev(9.1, false), &aud, &strict).0, Some(true));
    }
}
