mod cli;

use tkd_core::{accounts, config, footage, ideate, live, music, publish, recreate, render, review, scout, state, study, watch};

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Cmd};

/// Pilha da thread principal. O future do `match` de comandos passa de 1 MB em debug (pilha padrao do Windows)
/// e estourava ate no `--help`.
const MAIN_STACK: usize = 16 << 20;

fn main() -> Result<()> {
    std::thread::Builder::new()
        .name("tkd-main".into())
        .stack_size(MAIN_STACK)
        .spawn(|| tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(Box::pin(run())))?
        .join()
        .map_err(|_| anyhow::anyhow!("a thread principal do tkd caiu"))?
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TKD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args = Cli::parse();
    // CLI fechada (ou console fechado) = filhos morrem junto, igual ao app
    tkd_core::proc::adopt_current_process();

    match args.cmd {
        Cmd::Status => status()?,
        Cmd::Doctor => {
            let items = Box::pin(tkd_core::setup::check()).await;
            for i in &items {
                let mark = match i.status {
                    tkd_core::setup::Status::Ok => "ok  ",
                    tkd_core::setup::Status::Warn => "aten",
                    tkd_core::setup::Status::Missing => "FALT",
                };
                println!("[{mark}] {:<42} {}", i.label, i.detail);
                if i.status != tkd_core::setup::Status::Ok {
                    if let Some(c) = &i.command {
                        println!("       -> {c}");
                    }
                }
            }
            if tkd_core::setup::needs_setup(&items) {
                std::process::exit(1);
            }
        }
        Cmd::Usage { days } => {
            let store = state::Store::open()?;
            let n = tkd_core::usage::import_history(&store)?;
            if n > 0 {
                println!("{n} sessoes importadas do ~/.claude");
            }
            let s = tkd_core::usage::summary(&store, days)?;
            let fmt = |r: &tkd_core::state::UsageRow| {
                let tot = r.input + r.output + r.cache_read + r.cache_write;
                format!(
                    "{:>5} chamadas  {:>12} tokens  ({:>4.0}% cache)  {:>6} turnos",
                    r.calls, tot, if tot > 0 { r.cache_read as f64 * 100.0 / tot as f64 } else { 0.0 }, r.turns
                )
            };
            println!("TOTAL              {}", fmt(&s.total));
            println!("
por canal x tarefa:");
            for r in &s.by_pair {
                println!("  {:<14} {:<11} {}", r.channel, r.task, fmt(r));
            }
        }
        Cmd::Login { channel, timeout_min } => {
            let goals = match channel.as_deref() {
                Some(s) => vec![config::get_channel(s)?],
                // ja vinculado = ja tem @handle gravado
                None => config::load_channels()?
                    .into_iter()
                    .filter(|c| c.post_handle.is_empty())
                    .collect(),
            };
            if goals.is_empty() {
                println!("todos os canais ja tem conta vinculada — use `tkd accounts --check`");
            }
            for ch in goals {
                println!("--- login do canal {} (ate {timeout_min} min) ---", ch.slug);
                match accounts::login(&ch, timeout_min).await {
                    Ok(h) if !h.is_empty() => println!("[{}] vinculado a @{h}", ch.slug),
                    Ok(_) => println!("[{}] sessao salva (handle nao detectado)", ch.slug),
                    Err(e) => tracing::error!("[{}] {e}", ch.slug),
                }
            }
        }
        Cmd::Accounts { check, channel } => accounts::list(check, channel.as_deref()).await?,
        Cmd::Scout { channel, limit } => {
            let store = state::Store::open()?;
            for ch in config::select(channel.as_deref())? {
                match scout::scout(&ch, &store, limit).await {
                    Ok(n) => println!("[{}] {n} referencias novas analisadas", ch.slug),
                    Err(e) => tracing::error!("[{}] {e}", ch.slug),
                }
            }
        }
        Cmd::Logout { channel, platform } => {
            let ch = config::get_channel(&channel)?;
            accounts::logout_on(&ch, &platform).await?;
            println!("{channel}: {platform} desconectado");
        }
        Cmd::YtLogin { timeout_min } => {
            let v = footage::yt_login(timeout_min).await?;
            println!("cookies do YouTube prontos: {} ({} cookies)", v["file"].as_str().unwrap_or("?"), v["n"]);
        }
        Cmd::Footage { channel, topic, queries, per_query, speech, character } => {
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let n = footage::fetch(&ch, &store, topic.as_deref(), queries, per_query, speech.as_deref(), character.as_deref()).await?;
            println!("[{}] {n} clipes novos no banco", ch.slug);
        }
        Cmd::Catalog { channel, refresh } => {
            let ch = config::get_channel(&channel)?;
            let mut bank = footage::load_bank(&ch);
            footage::catalog_bank_with(&ch, &mut bank, refresh).await?;
            footage::save_bank(&ch, &bank)?;
            let n = bank.iter().filter(|c| !c.frames.is_empty()).count();
            println!("[{}] {n}/{} clipes com catalogo por frame", ch.slug, bank.len());
        }
        Cmd::Moments { file, url, channel, profile, speech, top } => {
            footage::inspect_clip(file, url, channel.as_deref(), profile, speech, top).await?;
        }
        Cmd::Live { url, minutes, threshold, min, max, lang, no_chat, captions } => {
            live::run_live(live::LiveOpts { url, minutes, threshold, min, max, lang, no_chat, captions, lazy: false, games: vec![], channel: String::new() }).await?;
        }
        Cmd::Music { channel, query } => {
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let t = music::fetch(&ch, &store, &query).await?;
            println!("{} ({:.0} bpm, {} batidas)", t.file, t.bpm, t.beats.len());
        }
        Cmd::Analyze { channel } => {
            let store = state::Store::open()?;
            for ch in config::select(channel.as_deref())? {
                match study::reanalyze(&ch, &store).await {
                    Ok(n) => println!("[{}] {n} referencias reanalisadas", ch.slug),
                    Err(e) => tracing::error!("[{}] {e}", ch.slug),
                }
            }
        }
        Cmd::Study { channel, force } => {
            let store = state::Store::open()?;
            for ch in config::select(channel.as_deref())? {
                if let Err(e) = study::study(&ch, &store, force).await {
                    tracing::error!("[{}] {e}", ch.slug);
                }
            }
        }
        Cmd::Ideate { channel, theme } => {
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let spec = ideate::ideate(&ch, &store, theme.as_deref()).await?;
            println!("{}", ideate::spec_path(&spec.id).display());
        }
        Cmd::Render { channel, id } => {
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let id = id
                .or_else(|| ideate::latest_id(&ch))
                .ok_or_else(|| anyhow::anyhow!("nenhuma spec para {channel}"))?;
            let out = render::render(&ch, &store, &id).await?;
            println!("{}", out.display());
        }
        Cmd::Review { channel, id } => {
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let id = id
                .or_else(|| ideate::latest_id(&ch))
                .ok_or_else(|| anyhow::anyhow!("nenhuma spec para {channel}"))?;
            let v = review::review(&ch, &store, &id).await?;
            println!("nota {} | manter: {} | {}", v.score, v.keep, v.problems.join("; "));
        }
        Cmd::Publish { channel, id, dry_run, platform, mark_posted, retry_unconfirmed, test_post } => {
            if test_post {
                std::env::set_var("TKD_TEST_POST", "1");
            }
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            let id = id
                .or_else(|| ideate::latest_id(&ch))
                .ok_or_else(|| anyhow::anyhow!("nenhuma spec para {channel}"))?;
            if let Some(net) = platform.as_deref().filter(|_| mark_posted.is_some() || retry_unconfirmed) {
                publish::resolve(&store, &ch, net, &id, mark_posted.as_deref())?;
                println!("{}", if mark_posted.is_some() { "gravado como postado" } else { "liberado para enviar de novo" });
                return Ok(());
            }
            let url = publish::publish(&ch, &store, &id, dry_run).await?;
            println!("{}", if url.is_empty() { "ok".into() } else { url });
        }
        Cmd::Run { channel, theme, dry_run, test_post } => {
            if test_post {
                std::env::set_var("TKD_TEST_POST", "1");
            }
            let store = state::Store::open()?;
            let ch = config::get_channel(&channel)?;
            tkd_core::events::enable_persistence();
            let r = watch::run_channel(&ch, &store, theme.as_deref(), dry_run).await;
            tkd_core::events::flush();
            let r = r?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Cmd::Recreate { channel, reference, script, script_file, end, voice_id, plan, no_compare, voice, lang, character, motion } => {
            let ch = config::get_channel(&channel)?;
            let script = match script_file {
                Some(p) => std::fs::read_to_string(&p)?,
                None => script.unwrap_or_default(),
            };
            let voice_id = voice_id
                .or_else(|| std::env::var("TKD_VOICE_ID").ok())
                .unwrap_or_else(|| "nPczCjzI2devNBz1zQrb".into());
            let out = recreate::recreate(
                &ch,
                recreate::Opts { ref_: reference, script, end_s: end, voice_id, plan, compare: !no_compare, voice, lang, character, motion },
            )
            .await?;
            println!("{}", out.display());
        }
        Cmd::Chaos { cmd } => chaos(cmd).await?,
        Cmd::Watch { once, every_hours, dry_run } => {
            tkd_core::events::enable_persistence();
            let store = state::Store::open()?;
            let r = watch::watch(store, once, every_hours, dry_run).await;
            tkd_core::events::flush();
            r?;
        }
    }
    Ok(())
}

async fn chaos(cmd: cli::ChaosCmd) -> Result<()> {
    use tkd_core::chaos_lab;
    match cmd {
        cli::ChaosCmd::Check => print!("{}", chaos_lab::describe(&chaos_lab::snapshot()?)),
        cli::ChaosCmd::List => {
            for s in chaos_lab::SCENARIOS {
                println!("{:<18} {}
{:<18} TKD_CHAOS={}
", s.name, s.hypothesis, "", s.chaos);
            }
        }
        cli::ChaosCmd::WatchLive { url, minutes, tick, games } => {
            tkd_core::clips::watch_one(
                &url,
                &games,
                std::time::Duration::from_secs_f64(minutes * 60.0),
                std::time::Duration::from_secs(tick.max(5)),
            )
            .await?;
        }
        cli::ChaosCmd::Run { scenario, channel, id, seed } => {
            anyhow::ensure!(!tkd_core::chaos::enabled(), "rode o `tkd chaos run` sem TKD_CHAOS: o cenario liga o macaco so no filho");
            let sc = chaos_lab::scenario(&scenario)
                .ok_or_else(|| anyhow::anyhow!("cenario desconhecido: {scenario} (veja `tkd chaos list`)"))?;
            config::get_channel(&channel)?;
            let exe = std::env::current_exe()?;
            let (passed, path) =
                chaos_lab::run(sc, chaos_lab::RunOpts { exe, channel: &channel, id: id.as_deref(), seed }).await?;
            println!("{}: {}", sc.name, if passed { "PASSOU" } else { "FALHOU" });
            println!("relatorio: {}", chaos_lab::rel(&path));
            if !passed {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn status() -> Result<()> {
    let store = state::Store::open()?;
    let channels = config::load_channels()?;
    println!("raiz: {}", config::root().display());
    println!("PC: {}", tkd_core::idle::describe());
    println!();
    for ch in channels {
        let (total, analyzed) = store.count_refs(&ch.slug)?;
        let lib = std::fs::read_dir(ch.library_dir())
            .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count())
            .unwrap_or(0);
        let card = if ch.style_card().exists() { "sim" } else { "NAO" };
        let last = store.last_post_at(&ch.slug)?.unwrap_or_else(|| "nunca".into());
        let conta = if ch.post_handle.is_empty() {
            "SEM LOGIN".to_string()
        } else {
            format!("@{}", ch.post_handle)
        };
        println!("{:<12} ref @{}  ->  posta em {conta}", ch.slug, ch.ref_handle);
        println!("  refs: {total} ({analyzed} analisadas)   style-card: {card}   biblioteca: {lib}");
        println!("  ultimo post: {last}");
    }
    Ok(())
}
