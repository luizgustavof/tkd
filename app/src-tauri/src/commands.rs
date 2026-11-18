use crate::dto::*;
use crate::error::{AppError, AppResult};
use crate::AppState;
use base64::Engine;
use std::sync::Arc;
use std::time::Duration;
use tauri::ipc::Channel as IpcChannel;
use tauri::State;
use tkd_core::{accounts, agent, config, events, state::Store, usage, watch};

fn get(slug: &str) -> AppResult<config::Channel> {
    Ok(config::get_channel(slug)?)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await?
}

// ---------------------------------------------------------------- primeira execucao

/// Confere ferramentas e configuracao (ffmpeg, node, claude, yt-dlp, venv, Edge, tkd.toml, canais).
#[tauri::command]
#[specta::specta]
pub async fn setup_check() -> AppResult<Vec<SetupItem>> {
    Ok(tkd_core::setup::check().await.into_iter().map(SetupItem::from).collect())
}

/// Conserto de um clique de um item da conferencia (`SetupItem.fix`).
#[tauri::command]
#[specta::specta]
pub async fn setup_fix(id: String) -> AppResult<String> {
    Ok(tkd_core::setup::fix(&id).await?)
}

// ---------------------------------------------------------------- canais

#[tauri::command]
#[specta::specta]
pub async fn app_status(state: State<'_, AppState>) -> AppResult<AppStatus> {
    let g = config::load_global();
    Ok(AppStatus {
        root: config::root().display().to_string(),
        autopilot: state.watch.lock().map(|w| w.is_some()).unwrap_or(false),
        busy: watch::busy_channels(),
        model_label: agent::model_label(&g.agents.model),
        effort: g.agents.effort,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn list_channels(state: State<'_, AppState>) -> AppResult<Vec<ChannelSummary>> {
    let store = state.store.clone();
    blocking(move || {
        let busy = watch::busy_channels();
        let evs = events::snapshot();
        let gm = config::load_global().agents.model;
        Ok(config::load_all_channels()?.iter().map(|c| summary(c, &store, &busy, &evs, &gm)).collect())
    })
    .await
}

fn detail(ch: &config::Channel, store: &Store) -> AppResult<ChannelDetail> {
    let g = config::load_global();
    let busy = watch::busy_channels();
    let evs = events::snapshot();
    let (refs_total, refs_analyzed) = store.count_refs(&ch.slug)?;
    let (library, avg_score) = store.library_stats(&ch.slug)?;
    let style = std::fs::read_to_string(ch.style_card()).unwrap_or_default();
    let style_excerpt: String = style
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .take(4)
        .collect::<Vec<_>>()
        .join("\n");
    let runs = {
        let db = store.db();
        let mut st = db
            .prepare_cached(
                "SELECT run_id, status, detail, at, ended_at FROM runs WHERE channel = ?1 AND step = 'run' ORDER BY id DESC LIMIT 8",
            )
            .map_err(anyhow::Error::from)?;
        let rows = st
            .query_map([&ch.slug], |r| {
                Ok(RunDto { run_id: r.get(0)?, status: r.get(1)?, detail: r.get(2)?, at: r.get(3)?, ended_at: r.get(4)? })
            })
            .map_err(anyhow::Error::from)?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };
    Ok(ChannelDetail {
        summary: summary(ch, store, &busy, &evs, &g.agents.model),
        persona: ch.persona.clone(),
        brand: tkd_core::brand::brand_of(&ch),
        format: if ch.is_edit() {
            "edit".into()
        } else if ch.is_clips() {
            config::FORMAT_CLIPS.into()
        } else {
            "texto".into()
        },
        streamers: ch.streamers.clone(),
        games: ch.games.clone(),
        watching: tkd_core::clips::watching(),
        characters: ch.characters.clone(),
        edit_lang: if ch.edit_lang.is_empty() { "auto".into() } else { ch.edit_lang.clone() },
        hashtags: ch.hashtags.clone(),
        ref_handle: ch.ref_handle.clone(),
        inspirations: ch.inspirations.clone(),
        notes: ch
            .notes
            .iter()
            .enumerate()
            .map(|(i, n)| NoteDto { index: i as u32, text: n.text.clone(), created_at: n.created_at.clone(), once: n.once })
            .collect(),
        schedule: ScheduleDto {
            days: ch.schedule.days.clone(),
            times: ch.schedule.times.clone(),
            jitter_min: ch.schedule.jitter_min,
        },
        agent_model: ch.agent.model.clone(),
        agent_effort: ch.agent.effort.clone(),
        picks: picks(Some(ch), &g),
        learned: Learned {
            has_style_card: !style.is_empty(),
            style_excerpt,
            refs_total: refs_total as i32,
            refs_analyzed: refs_analyzed as i32,
            refs_by_handle: store
                .refs_by_handle(&ch.slug)?
                .into_iter()
                .map(|(handle, studied, seen)| HandleRefs { handle, studied: studied as i32, seen: seen as i32 })
                .collect(),
            library: library as i32,
            avg_score,
            last_problems: store.recent_review_problems(&ch.slug, 3)?,
        },
        runs,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_channel(state: State<'_, AppState>, slug: String) -> AppResult<ChannelDetail> {
    let store = state.store.clone();
    blocking(move || detail(&get(&slug)?, &store)).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_channel(input: NewChannel) -> AppResult<String> {
    let slug = input.slug.trim().to_lowercase();
    let ch = config::create_channel(&slug, &input.ref_handle, input.niche.trim(), "")?;
    Ok(ch.slug)
}

#[tauri::command]
#[specta::specta]
pub async fn update_channel(slug: String, patch: ChannelPatch) -> AppResult<()> {
    config::edit_channel(&slug, |doc| {
        if let Some(n) = patch.niche {
            doc["niche"] = toml_edit::value(n.trim());
        }
        if let Some(p) = patch.persona {
            doc["persona"] = toml_edit::value(p.trim());
        }
        if let Some(b) = patch.brand {
            doc["brand"] = toml_edit::value(b.trim());
        }
        if let Some(f) = patch.format {
            let f = match f.as_str() {
                "edit" => "edit",
                "clips" | "cortes" => config::FORMAT_CLIPS,
                _ => "texto",
            };
            doc["format"] = toml_edit::value(f);
        }
        if let Some(c) = patch.characters {
            let mut arr = toml_edit::Array::new();
            for x in c.iter().map(|x| x.trim()).filter(|x| !x.is_empty()) {
                arr.push(x);
            }
            doc["characters"] = toml_edit::value(arr);
        }
        if let Some(list) = patch.games {
            let mut arr = toml_edit::Array::new();
            for g in list.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
                arr.push(g);
            }
            doc["games"] = toml_edit::value(arr);
        }
        if let Some(list) = patch.streamers {
            let mut arr = toml_edit::Array::new();
            for u in list.iter().map(|u| u.trim()).filter(|u| u.starts_with("http")) {
                arr.push(u);
            }
            doc["streamers"] = toml_edit::value(arr);
        }
        if let Some(l) = patch.edit_lang {
            let l = if ["auto", "en", "ja", "pt"].contains(&l.as_str()) { l } else { "auto".into() };
            doc["edit_lang"] = toml_edit::value(l);
        }
        if let Some(h) = patch.hashtags {
            let mut arr = toml_edit::Array::new();
            for t in h.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                arr.push(if t.starts_with('#') { t.to_string() } else { format!("#{t}") });
            }
            doc["hashtags"] = toml_edit::value(arr);
        }
        Ok(())
    })?;
    Ok(())
}

fn write_inspirations(slug: &str, list: Vec<String>) -> AppResult<()> {
    config::edit_channel(slug, |doc| {
        let mut arr = toml_edit::Array::new();
        for h in list {
            arr.push(h);
        }
        doc["inspirations"] = toml_edit::value(arr);
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn add_inspiration(slug: String, handle: String) -> AppResult<()> {
    let ch = get(&slug)?;
    let h = config::clean_handle(&handle);
    if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
        return Err(AppError::invalid("Use o @ do canal, por exemplo @seu.canal"));
    }
    if ch.ref_handles().iter().any(|x| x.eq_ignore_ascii_case(&h)) {
        return Err(AppError::busy(format!("@{h} já está na lista")));
    }
    // sem referencia principal: a primeira inspiracao vira a principal
    if ch.ref_handle.is_empty() || ch.ref_handle == "none" {
        config::edit_channel(&slug, |doc| {
            doc["ref_handle"] = toml_edit::value(h.clone());
            Ok(())
        })?;
        return Ok(());
    }
    let mut list = ch.inspirations.clone();
    list.push(h);
    write_inspirations(&slug, list)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_inspiration(slug: String, handle: String) -> AppResult<()> {
    let ch = get(&slug)?;
    let h = config::clean_handle(&handle);
    if ch.ref_handle.eq_ignore_ascii_case(&h) {
        // promove a proxima inspiracao a principal
        let mut rest = ch.inspirations.clone();
        let next = if rest.is_empty() { String::new() } else { rest.remove(0) };
        config::edit_channel(&slug, |doc| {
            doc["ref_handle"] = toml_edit::value(next);
            Ok(())
        })?;
        return write_inspirations(&slug, rest);
    }
    let list = ch.inspirations.into_iter().filter(|x| !x.eq_ignore_ascii_case(&h)).collect();
    write_inspirations(&slug, list)
}

fn write_notes(slug: &str, notes: &[config::Note]) -> AppResult<()> {
    config::edit_channel(slug, |doc| {
        doc.remove("notes");
        if !notes.is_empty() {
            config::set_item(doc, "notes", config::to_item(&notes)?);
        }
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn add_note(slug: String, text: String, once: bool) -> AppResult<()> {
    let t = text.trim();
    if t.is_empty() {
        return Err(AppError::invalid("Escreva a observação"));
    }
    let mut notes = get(&slug)?.notes;
    notes.push(config::Note { text: t.to_string(), created_at: tkd_core::state::now(), once });
    write_notes(&slug, &notes)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_note(slug: String, index: u32) -> AppResult<()> {
    let mut notes = get(&slug)?.notes;
    if (index as usize) < notes.len() {
        notes.remove(index as usize);
    }
    write_notes(&slug, &notes)
}

#[tauri::command]
#[specta::specta]
pub async fn set_schedule(slug: String, schedule: ScheduleDto) -> AppResult<()> {
    let mut s = config::Schedule {
        days: schedule.days,
        times: schedule.times,
        jitter_min: schedule.jitter_min.min(120),
        ..Default::default()
    };
    s.normalize();
    if s.times.is_empty() {
        return Err(AppError::invalid("Escolha pelo menos um horário"));
    }
    config::edit_channel(&slug, |doc| {
        config::set_item(doc, "schedule", config::to_item(&s)?);
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn pause_channel(slug: String, paused: bool) -> AppResult<()> {
    watch::set_paused(&slug, paused)?;
    Ok(())
}

/// Troca o nome do canal (pastas, logins e historico vao junto). Devolve o slug novo.
#[tauri::command]
#[specta::specta]
pub async fn rename_channel(state: State<'_, AppState>, slug: String, new_slug: String) -> AppResult<String> {
    get(&slug)?;
    let new_slug = new_slug.trim().to_lowercase();
    let login_open = state.logins.lock().map(|m| m.keys().any(|k| k.starts_with(&format!("{slug}:")))).unwrap_or(false);
    if login_open {
        return Err(AppError::busy("Feche a janela de login desse canal antes de trocar o nome."));
    }
    if watch::busy_channels().contains(&slug) {
        return Err(AppError::busy(format!("{slug} está rodando agora. Espere terminar pra trocar o nome.")));
    }
    let ch = tkd_core::config::rename_channel(&slug, &new_slug)?;
    state.store.rename_channel(&slug, &ch.slug)?;
    Ok(ch.slug)
}

/// Move o canal e o perfil do navegador pra `.lixeira/`. Recusa se o canal estiver rodando.
#[tauri::command]
#[specta::specta]
pub async fn delete_channel(state: State<'_, AppState>, slug: String) -> AppResult<String> {
    get(&slug)?;
    let login_open = state.logins.lock().map(|m| m.keys().any(|k| k.starts_with(&format!("{slug}:")))).unwrap_or(false);
    if login_open {
        return Err(AppError::busy("Feche a janela de login desse canal antes de excluir."));
    }
    if watch::busy_channels().contains(&slug) {
        return Err(AppError::busy(format!("{slug} está rodando agora. Espere terminar pra excluir.")));
    }
    let dst = tkd_core::config::delete_channel(&slug)?;
    state.store.forget_channel(&slug)?;
    Ok(dst.display().to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn avatar(slug: String, platform: String) -> AppResult<Option<String>> {
    let ch = get(&slug)?;
    for p in [platform.as_str(), "tiktok", "youtube"] {
        if let Ok(bytes) = std::fs::read(ch.avatar(p)) {
            let mime = if bytes.starts_with(&[0x89, b'P']) { "image/png" } else if bytes.starts_with(b"RIFF") { "image/webp" } else { "image/jpeg" };
            return Ok(Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------- contas

#[tauri::command]
#[specta::specta]
pub async fn login(state: State<'_, AppState>, slug: String, platform: String) -> AppResult<String> {
    let ch = get(&slug)?;
    if !accounts::PLATFORMS.contains(&platform.as_str()) {
        return Err(AppError::invalid("Plataforma desconhecida"));
    }
    let key = format!("{slug}:{platform}");
    let task = tokio::spawn(async move { accounts::login_on(&ch, &platform, 10).await });
    if let Ok(mut m) = state.logins.lock() {
        if m.contains_key(&key) {
            task.abort();
            return Err(AppError::busy("Já tem uma janela de login aberta para essa conta"));
        }
        m.insert(key.clone(), task.abort_handle());
    }
    let r = task.await;
    if let Ok(mut m) = state.logins.lock() {
        m.remove(&key);
    }
    match r {
        Ok(r) => Ok(r?),
        Err(e) if e.is_cancelled() => Err(AppError::new("cancelado", "Login cancelado")),
        Err(e) => Err(e.into()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_login(state: State<'_, AppState>, slug: String, platform: String) -> AppResult<()> {
    if let Ok(mut m) = state.logins.lock() {
        if let Some(h) = m.remove(&format!("{slug}:{platform}")) {
            h.abort();
        }
    }
    Ok(())
}

/// Sai de uma plataforma no perfil do canal (so os cookies dela; a outra continua logada).
#[tauri::command]
#[specta::specta]
pub async fn logout(slug: String, platform: String) -> AppResult<()> {
    let ch = get(&slug)?;
    if !accounts::PLATFORMS.contains(&platform.as_str()) {
        return Err(AppError::invalid("Plataforma desconhecida"));
    }
    accounts::logout_on(&ch, &platform).await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn check_session(slug: String, platform: String) -> AppResult<SessionCheck> {
    let ch = get(&slug)?;
    let w = accounts::whoami_on(&ch, &platform).await?;
    if w.logged_in && !w.handle.is_empty() {
        accounts::set_account(&ch, &platform, &w.handle)?;
    }
    Ok(SessionCheck { logged_in: w.logged_in, handle: w.handle })
}

// ---------------------------------------------------------------- pipeline

#[tauri::command]
#[specta::specta]
pub async fn run_test_video(state: State<'_, AppState>, slug: String) -> AppResult<()> {
    let ch = get(&slug)?;
    if watch::busy_channels().contains(&slug) {
        return Err(AppError::busy("Esse canal já está fazendo um vídeo"));
    }
    let store = state.store.clone();
    tokio::spawn(async move {
        if let Err(e) = watch::run_channel(&ch, &store, None, true).await {
            tracing::error!("[{}] video de teste: {e:#}", ch.slug);
        }
    });
    Ok(())
}

/// Post de teste: faz um video e publica de verdade, mas so o canal: TikTok "Somente voce" e YouTube nao listado.
/// Nao conta no limite do dia, nos horarios nem no intervalo entre posts. Os outros canais seguem publicos.
#[tauri::command]
#[specta::specta]
pub async fn run_test_post(state: State<'_, AppState>, slug: String) -> AppResult<()> {
    let ch = get(&slug)?;
    if ch.accounts.tiktok.is_empty() && ch.accounts.youtube.is_empty() {
        return Err(AppError::invalid("Conecte o TikTok ou o YouTube do canal antes do post de teste"));
    }
    if watch::busy_channels().contains(&slug) {
        return Err(AppError::busy("Esse canal já está fazendo um vídeo"));
    }
    let Some(guard) = tkd_core::publish::TestPost::begin(&slug) else {
        return Err(AppError::busy("Esse canal já está num post de teste"));
    };
    let store = state.store.clone();
    tokio::spawn(async move {
        let _guard = guard; // modo teste ate o fim (inclusive com erro)
        match watch::run_channel(&ch, &store, None, false).await {
            Ok(_) => tracing::info!("[{}] post de teste terminou (Somente voce / nao listado)", ch.slug),
            Err(e) => tracing::error!("[{}] post de teste: {e:#}", ch.slug),
        }
    });
    Ok(())
}

/// Busca os videos novos dos perfis estudados e reescreve o estilo do canal (sem fazer video).
#[tauri::command]
#[specta::specta]
pub async fn study_now(state: State<'_, AppState>, slug: String, handle: Option<String>) -> AppResult<()> {
    let ch = get(&slug)?;
    if watch::busy_channels().contains(&slug) {
        return Err(AppError::busy("Esse canal já está trabalhando agora"));
    }
    if ch.ref_handles().is_empty() {
        return Err(AppError::invalid("Adicione pelo menos um perfil pra estudar"));
    }
    let store = state.store.clone();
    tokio::spawn(async move {
        if let Err(e) = watch::study_now(&ch, &store, handle.as_deref()).await {
            tracing::error!("[{}] estudo: {e:#}", ch.slug);
        }
    });
    Ok(())
}

/// Abre um link no navegador padrao (o webview do app nao abre target=_blank).
/// So http(s): nada de file:, javascript: ou caminhos locais.
#[tauri::command]
#[specta::specta]
pub async fn open_url(url: String) -> AppResult<()> {
    let u = url.trim();
    let ok = (u.starts_with("https://") || u.starts_with("http://")) && !u.chars().any(|c| c.is_whitespace() || c == '"');
    if !ok {
        return Err(AppError::invalid("Link inválido"));
    }
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(u).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(u).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(u).spawn();
    r.map(|_| ()).map_err(|e| AppError::invalid(&format!("Não consegui abrir o navegador: {e}")))
}

#[tauri::command]
#[specta::specta]
pub async fn watch_start(state: State<'_, AppState>) -> AppResult<()> {
    crate::start_watch(&state);
    let mut g = config::load_global();
    g.app.autopilot = true;
    config::save_global(&g)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn watch_stop(state: State<'_, AppState>) -> AppResult<()> {
    if let Ok(mut w) = state.watch.lock() {
        if let Some(c) = w.take() {
            c.cancel();
        }
    }
    let mut g = config::load_global();
    g.app.autopilot = false;
    config::save_global(&g)?;
    Ok(())
}

// ---------------------------------------------------------------- agentes

#[tauri::command]
#[specta::specta]
pub async fn get_agent_config() -> AppResult<AgentConfig> {
    let g = config::load_global();
    let channels = config::load_all_channels()?
        .iter()
        .map(|c| ChannelAgent {
            slug: c.slug.clone(),
            color: color_for(&c.slug),
            model: c.agent.model.clone(),
            effort: c.agent.effort.clone(),
            tasks: task_rows(&c.agent.tasks),
            picks: picks(Some(c), &g),
        })
        .collect();
    Ok(AgentConfig {
        model: g.agents.model.clone(),
        effort: g.agents.effort.clone(),
        parallel: g.agents.parallel as u32,
        tasks: task_rows(&g.tasks),
        channels,
        models: agent::MODELS.iter().map(|(v, l)| Opt { value: v.to_string(), label: l.to_string() }).collect(),
        efforts: agent::EFFORTS.iter().map(|e| e.to_string()).collect(),
    })
}

fn check_pick(model: &str, effort: &str) -> AppResult<()> {
    if !model.is_empty() && !agent::MODELS.iter().any(|(v, _)| *v == model) {
        return Err(AppError::invalid("Modelo desconhecido"));
    }
    if !effort.is_empty() && !agent::EFFORTS.contains(&effort) {
        return Err(AppError::invalid("Esforço desconhecido"));
    }
    Ok(())
}

/// Grava modelo/esforco num nivel. Vazio = herda do nivel de cima.
#[tauri::command]
#[specta::specta]
pub async fn set_agent_config(scope: AgentScope, model: String, effort: String) -> AppResult<()> {
    check_pick(&model, &effort)?;
    if let Some(t) = &scope.task {
        if !agent::TASKS.iter().any(|(k, _)| k == t) {
            return Err(AppError::invalid("Tarefa desconhecida"));
        }
    }
    let pick = config::AgentPick { model: model.clone(), effort: effort.clone() };
    match (&scope.channel, &scope.task) {
        (None, None) => {
            if model.is_empty() || effort.is_empty() {
                return Err(AppError::invalid("O padrão precisa de modelo e esforço"));
            }
            let mut g = config::load_global();
            g.agents.model = model;
            g.agents.effort = effort;
            config::save_global(&g)?;
        }
        (None, Some(t)) => {
            let mut g = config::load_global();
            if pick == config::AgentPick::default() {
                g.tasks.remove(t);
            } else {
                g.tasks.insert(t.clone(), pick);
            }
            config::save_global(&g)?;
        }
        (Some(slug), task) => {
            let mut cfg = get(slug)?.agent;
            match task {
                None => {
                    cfg.model = model;
                    cfg.effort = effort;
                }
                Some(t) => {
                    if pick == config::AgentPick::default() {
                        cfg.tasks.remove(t);
                    } else {
                        cfg.tasks.insert(t.clone(), pick);
                    }
                }
            }
            config::edit_channel(slug, |doc| {
                doc.remove("agent");
                if cfg != config::AgentCfg::default() {
                    config::set_item(doc, "agent", config::to_item(&cfg)?);
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_parallel(n: u32) -> AppResult<()> {
    let mut g = config::load_global();
    g.agents.parallel = n.clamp(1, 8) as usize;
    config::save_global(&g)?;
    tkd_core::agent::set_parallel(g.agents.parallel);
    Ok(())
}

// ---------------------------------------------------------------- consumo

fn insights(s: &tkd_core::usage::Summary) -> Vec<String> {
    let mut out = Vec::new();
    // releitura de cache medida so nos canais (as conversas distorcem a conta)
    let mut t = tkd_core::state::UsageRow::default();
    for r in s.by_channel.iter().filter(|r| r.channel != usage::INTERACTIVE) {
        t.input += r.input;
        t.output += r.output;
        t.cache_read += r.cache_read;
        t.cache_write += r.cache_write;
    }
    let t = &t;
    let all = &s.total;
    let tot = (t.input + t.output + t.cache_read + t.cache_write) as f64;
    let tot_all = (all.input + all.output + all.cache_read + all.cache_write) as f64;
    if tot > 0.0 {
        out.push(format!(
            "{:.0}% do consumo é releitura de contexto (cache), não texto novo.",
            t.cache_read as f64 * 100.0 / tot
        ));
    }
    let channels: Vec<_> = s.by_pair.iter().filter(|r| r.channel != usage::INTERACTIVE).collect();
    if let Some(top) = channels.first() {
        let per = (top.input + top.output + top.cache_read + top.cache_write) as f64 / top.calls.max(1) as f64;
        out.push(format!(
            "{} no {} é o mais caro: {} por chamada, {:.1} turnos em média.",
            agent::task_label(&top.task),
            top.channel,
            human(per),
            top.turns as f64 / top.calls.max(1) as f64
        ));
    }
    if let Some(i) = s.by_channel.iter().find(|r| r.channel == usage::INTERACTIVE) {
        let it = (i.input + i.output + i.cache_read + i.cache_write) as f64;
        if tot_all > 0.0 && it / tot_all > 0.5 {
            out.push(format!("As conversas fora dos canais são {:.0}% de tudo que o Claude gastou nesta pasta.", it * 100.0 / tot_all));
        }
    }
    out
}

fn human(n: f64) -> String {
    if n >= 1e6 {
        format!("{:.1} mi", n / 1e6).replace('.', ",")
    } else if n >= 1e3 {
        format!("{:.0} mil", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

#[tauri::command]
#[specta::specta]
pub async fn usage_summary(state: State<'_, AppState>, days: i32) -> AppResult<UsageSummary> {
    let store = state.store.clone();
    blocking(move || {
        if let Err(e) = usage::import_history(&store) {
            tracing::warn!("importando historico: {e}");
        }
        let s = usage::summary(&store, days as i64)?;
        let conv = |v: &Vec<tkd_core::state::UsageRow>| v.iter().map(UsageRowDto::from).collect::<Vec<_>>();
        Ok(UsageSummary {
            days,
            total: UsageRowDto::from(&s.total),
            by_channel: conv(&s.by_channel),
            by_task: conv(&s.by_task),
            by_pair: conv(&s.by_pair),
            by_model: conv(&s.by_model),
            daily: s.daily.iter().map(|(d, f, c)| DayUsage { day: d.clone(), fresh: *f as f64, cache_read: *c as f64 }).collect(),
            insights: insights(&s),
            before_after: usage::before_after(&store)?
                .into_iter()
                .map(|c| TaskCompareDto {
                    task_label: tkd_core::agent::task_label(&c.task).to_string(),
                    task: c.task,
                    before_calls: c.before_calls as f64,
                    before_avg: c.before_avg as f64,
                    after_calls: c.after_calls as f64,
                    after_avg: c.after_avg as f64,
                })
                .collect(),
        })
    })
    .await
}

// ---------------------------------------------------------------- ao vivo

/// Abre o fluxo de eventos ao vivo: estado inicial e depois lotes (no maximo ~10 por segundo).
#[tauri::command]
#[specta::specta]
pub async fn live_subscribe(state: State<'_, AppState>, on_event: IpcChannel<LiveBatch>) -> AppResult<()> {
    let mut rx = events::subscribe();
    let store: Arc<Store> = state.store.clone();
    let mut initial: Vec<EventDto> = {
        let s = store.clone();
        blocking(move || Ok(s.recent_events(None, 400)?)).await?.iter().map(EventDto::from).collect()
    };
    // o snapshot em memoria cobre o que ainda nao foi gravado
    for e in events::snapshot() {
        let d = EventDto::from(&e);
        if !initial.iter().any(|x| x.at == d.at && x.channel == d.channel && x.step == d.step && x.status == d.status) {
            initial.push(d);
        }
    }
    initial.sort_by(|a, b| a.at.cmp(&b.at));
    on_event.send(LiveBatch { initial: true, events: initial })?;

    let task = tokio::spawn(async move {
        loop {
            let first = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            let mut batch = vec![EventDto::from(&first)];
            let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                // progresso repetido da mesma etapa: manda so o ultimo
                let d = EventDto::from(&e);
                if d.status == "progress" && d.action.is_empty() {
                    if let Some(p) = batch.iter_mut().rev().find(|p| p.status == "progress" && p.channel == d.channel && p.step == d.step) {
                        *p = d;
                        continue;
                    }
                }
                batch.push(d);
            }
            if on_event.send(LiveBatch { initial: false, events: batch }).is_err() {
                return; // a tela fechou
            }
        }
    });
    // reload da webview (F5/HMR) assina de novo; o fluxo velho seguia vivo serializando todo evento
    if let Some(old) = state.live.lock().unwrap_or_else(|e| e.into_inner()).replace(task.abort_handle()) {
        old.abort();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn browser_status() -> AppResult<BrowserStatus> {
    let (h, w) = tkd_core::browser::status();
    Ok(BrowserStatus {
        holder: h.map(|h| BrowserHolder { channel: h.channel, script: h.script, since: h.since }),
        waiting: w,
    })
}

// ---------------------------------------------------------------- cortes gravados

#[tauri::command]
#[specta::specta]
pub async fn list_clips() -> AppResult<ClipsView> {
    blocking(|| {
        let clips = tkd_core::clips::all_clips(400)
            .into_iter()
            .map(|c| ClipDto {
                key: c.key,
                mp4: c.mp4,
                vertical: c.vertical,
                platform: c.platform,
                streamer: c.streamer,
                url: c.url,
                score: c.score,
                duration: c.duration,
                created: c.created,
                category: c.category,
                chat: c.chat,
                transcript: c.transcript,
                used: c.used,
            })
            .collect();
        Ok(ClipsView { watching: tkd_core::clips::watching(), max_watch: tkd_core::clips::max_watch() as u32, clips })
    })
    .await
}

/// Abre o mp4 de um corte no player padrao (`reveal`: mostra na pasta). So arquivos .mp4 dentro de out/live.
#[tauri::command]
#[specta::specta]
pub async fn open_clip(path: String, reveal: bool) -> AppResult<()> {
    let base = config::root().join("out").join("live");
    let p = std::path::PathBuf::from(&path);
    let ok = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        && match (p.canonicalize(), base.canonicalize()) {
            (Ok(p), Ok(b)) => p.starts_with(b),
            _ => false,
        };
    if !ok {
        return Err(AppError::invalid("Corte não encontrado"));
    }
    #[cfg(windows)]
    let r = if reveal {
        // explorer nao entende o /select com a aspa que o Rust poe no argumento inteiro
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn()
    } else {
        std::process::Command::new("explorer").arg(&p).spawn()
    };
    #[cfg(not(windows))]
    let r = {
        let _ = reveal;
        std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(&p).spawn()
    };
    r.map(|_| ()).map_err(|e| AppError::invalid(&format!("Não consegui abrir: {e}")))
}

/// Apaga um corte gravado (json + mp4s + legenda). Nao mexe em post ja publicado.
#[tauri::command]
#[specta::specta]
pub async fn delete_clip(key: String) -> AppResult<u32> {
    blocking(move || tkd_core::clips::delete_clip(&key).map(|n| n as u32).map_err(|e| AppError::invalid(&e.to_string()))).await
}
