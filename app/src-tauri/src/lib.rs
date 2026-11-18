//! App desktop do tkd: comandos Tauri em cima do tkd-core.

mod commands;
mod dto;
mod error;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tkd_core::{config, events, state::Store, watch};
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub store: Arc<Store>,
    /// piloto automatico ligado = Some
    pub watch: Mutex<Option<CancellationToken>>,
    /// janelas de login abertas ("canal:plataforma")
    pub logins: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// fluxo de eventos ao vivo da tela (um so: reload da webview abre outro e o velho tem que morrer)
    pub live: Mutex<Option<tokio::task::AbortHandle>>,
}

/// Liga o piloto automatico (idempotente).
pub fn start_watch(state: &AppState) {
    let Ok(mut w) = state.watch.lock() else { return };
    if w.is_some() {
        return;
    }
    let cancel = CancellationToken::new();
    *w = Some(cancel.clone());
    let store = state.store.clone();
    tauri::async_runtime::spawn(async move {
        let o = watch::WatchOpts { dry_run: false, scout_every_hours: 6, once: false };
        if let Err(e) = watch::watch_loop(store, o, cancel).await {
            tracing::error!("piloto automatico parou: {e:#}");
        }
    });
}

/// Pasta do projeto: $TKD_ROOT ou o primeiro pai (do cwd ou do exe) com channels/ e agents/.
fn find_root() -> PathBuf {
    if let Ok(r) = std::env::var("TKD_ROOT") {
        return PathBuf::from(r);
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    for s in starts.into_iter().flatten() {
        for d in s.ancestors() {
            if d.join("channels").is_dir() && d.join("agents").is_dir() {
                return d.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    use commands::*;
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        app_status,
        setup_check,
        setup_fix,
        list_channels,
        get_channel,
        create_channel,
        update_channel,
        add_inspiration,
        remove_inspiration,
        add_note,
        delete_note,
        set_schedule,
        pause_channel,
        delete_channel,
        rename_channel,
        avatar,
        login,
        cancel_login,
        logout,
        check_session,
        run_test_video,
        run_test_post,
        study_now,
        open_url,
        watch_start,
        watch_stop,
        get_agent_config,
        set_agent_config,
        set_parallel,
        usage_summary,
        live_subscribe,
        browser_status,
        list_clips,
        open_clip,
        delete_clip,
    ])
}

/// Gera app/src/bindings.ts (`tkd-app --export-bindings` e no `tauri dev`).
/// So grava quando muda: reescrever a toa faz o Vite recarregar a tela no meio do boot.
pub fn export_bindings(path: &str) -> anyhow::Result<()> {
    let tmp = std::env::temp_dir().join(format!("tkd-bindings-{}.ts", std::process::id()));
    specta_builder()
        .export(
            specta_typescript::Typescript::default()
                .header("// gerado por tauri-specta a partir de app/src-tauri — nao edite\n// @ts-nocheck"),
            &tmp,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let new = std::fs::read_to_string(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    if std::fs::read_to_string(path).ok().as_deref() != Some(new.as_str()) {
        std::fs::write(path, new)?;
    }
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TKD_LOG").unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let root = find_root();
    config::set_root(&root);
    tracing::info!("raiz do projeto: {}", root.display());

    #[cfg(debug_assertions)]
    if let Err(e) = export_bindings(&root.join("app").join("src").join("bindings.ts").to_string_lossy()) {
        tracing::warn!("bindings.ts: {e}");
    }

    let store = match Store::open() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("nao consegui abrir o banco: {e:#}");
            std::process::exit(1);
        }
    };
    match store.close_open_runs() {
        Ok(0) => {}
        Ok(n) => tracing::warn!("{n} video(s) ficaram pela metade na ultima vez: marcados como interrompidos"),
        Err(e) => tracing::warn!("nao fechei os runs abertos: {e}"),
    }
    // publicacao que morreu no meio: a proxima postagem do canal confere o perfil antes (publish::reconcile)
    match store.unconfirmed_publishes(None) {
        Ok(p) if !p.is_empty() => {
            for p in &p {
                tracing::warn!("publicacao sem confirmacao: {} {} {} desde {} ({})", p.channel, p.platform, p.video_id, p.started_at, p.detail);
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("nao li as publicacoes pendentes: {e}"),
    }
    tkd_core::proc::adopt_current_process();
    events::enable_persistence();

    let builder = specta_builder();
    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            // previa dos cortes na tela "Cortes": o webview so le mp4 de out/live (escopo aberto aqui porque a raiz e dinamica)
            let lives = config::root().join("out").join("live");
            let _ = std::fs::create_dir_all(&lives);
            if let Err(e) = app.asset_protocol_scope().allow_directory(&lives, true) {
                tracing::warn!("previa dos cortes desligada: {e}");
            }
            app.manage(AppState { store, watch: Mutex::new(None), logins: Mutex::new(HashMap::new()), live: Mutex::new(None) });
            if config::load_global().app.autopilot {
                start_watch(&app.state::<AppState>());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("erro no app: {e}");
            std::process::exit(1);
        })
        .run(|_, e| {
            // grava o ultimo lote (<= 400 ms) antes de sair: senao o "ok" final de uma etapa se perdia
            if let tauri::RunEvent::Exit = e {
                events::flush();
            }
        });
}
