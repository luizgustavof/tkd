//! Canal de CORTES de live (formato "clips", antes "cortes"): Twitch, Kick e YouTube.
//!
//! - vigia: com o piloto ligado, liga o clipper (`live::rodar`) quando um streamer do canal entra
//!   ao vivo (no maximo `max_watch()` lives ao mesmo tempo: e pesado na CPU);
//! - fila: cortes das ultimas `MAX_AGE_H` horas dos streamers do canal, maior nota do momento
//!   primeiro, sem repetir;
//! - avaliacao SEM o modelo local (ele so avalia edits): barreira tecnica medida no ffmpeg e,
//!   pros que passam, o Claude com transcricao + chat + quadros (nota, titulo, legenda, recorte);
//! - o resultado vira uma Spec comum (mp4 em out/renders/<id>.mp4) e segue pra review/publish,
//!   com credito ao streamer na legenda.

use crate::proc::TreeExt;
use crate::{agent, config::Channel, review::Verdict, spec::Spec, state::Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Lives assistidas ao mesmo tempo (todas as contas). Com `--lazy` a live so grava e mede (streamlink +
/// ffmpeg); whisper e 9:16 rodam so no corte escolhido, entao cabem varias. `TKD_MAX_WATCH` muda (1 a 12).
pub const MAX_WATCH: usize = 6;

pub fn max_watch() -> usize {
    std::env::var("TKD_MAX_WATCH").ok().and_then(|v| v.trim().parse().ok()).map(|n: usize| n.clamp(1, 12)).unwrap_or(MAX_WATCH)
}
/// Corte de live envelhece rapido.
pub const MAX_AGE_H: f64 = 48.0;
/// Quantos cortes no maximo passam pela barreira tecnica por video.
const MAX_TRIES: usize = 10;
/// Quantos cortes (ja aprovados na barreira) o Claude compara pra escolher o melhor.
const POOL: usize = 4;
/// Quadros por corte na comparacao.
const FRAMES_PER_CLIP: u32 = 4;

static WATCHING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// Proxima checagem de cada streamer offline: nao abre `streamlink --json` todo minuto.
static NEXT_CHECK: Mutex<Option<HashMap<String, std::time::Instant>>> = Mutex::new(None);
/// Cancela todas as capturas (piloto desligado): a arvore live.py + streamlink + ffmpeg morre junto.
static STOP: Mutex<Option<tokio_util::sync::CancellationToken>> = Mutex::new(None);

/// Quando a captura de cada streamer terminou pela ultima vez (live caiu ou acabou).
static ENDED: Mutex<Option<HashMap<String, std::time::Instant>>> = Mutex::new(None);
/// Live que acabou de cair costuma voltar logo (queda do streamer, troca de cena): confere mais vezes.
const JUST_ENDED: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Intervalo entre checagens de um streamer offline. YouTube custa requisicoes do orcamento do IP.
/// Logo depois de uma queda o intervalo cai para 1 min (2 no YouTube): antes a live voltava e o vigia so
/// reabria ate 3 min depois (+90 s de aquecimento do live.py = ~5 min sem corte a cada queda).
fn offline_backoff(key: &str) -> std::time::Duration {
    let yt = key.contains("youtube.com") || key.contains("youtu.be");
    let recent = ENDED
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(key).copied()))
        .is_some_and(|t| t.elapsed() < JUST_ENDED);
    let mins = match (yt, recent) {
        (true, true) => 2,
        (true, false) => 10,
        (false, true) => 1,
        (false, false) => 3,
    };
    std::time::Duration::from_secs(mins * 60)
}

fn stop_token() -> tokio_util::sync::CancellationToken {
    let mut g = STOP.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(tokio_util::sync::CancellationToken::new).clone()
}

/// Vigia uma URL so, no mesmo ritmo do piloto (uma conferencia por `tick`), por `total`; depois desliga e espera
/// a captura fechar. Para os experimentos de caos (`tkd chaos watch-live`): o piloto inteiro mexeria nos canais.
pub async fn watch_one(url: &str, games: &[String], total: std::time::Duration, tick: std::time::Duration) -> Result<()> {
    let ch: Channel = toml::from_str(&format!(
        "slug = \"_chaos-live\"
ref_handle = \"none\"
post_handle = \"\"
persona = \"\"
niche = \"\"
streamers = [{:?}]
",
        url
    ))
    .context("canal de teste")?;
    let mut ch = ch;
    ch.games = games.to_vec();
    let end_at = std::time::Instant::now() + total;
    while std::time::Instant::now() < end_at {
        ensure_watchers(&ch);
        tokio::time::sleep(tick.min(end_at.saturating_duration_since(std::time::Instant::now()))).await;
    }
    stop_watchers();
    for _ in 0..60 {
        let blank = WATCHING.lock().map(|g| g.as_ref().is_none_or(|s| s.is_empty())).unwrap_or(true);
        if blank {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
}

/// Desliga todas as capturas de live (chamado quando o piloto para).
pub fn stop_watchers() {
    if let Some(t) = STOP.lock().unwrap_or_else(|e| e.into_inner()).take() {
        t.cancel();
    }
    if let Ok(mut g) = NEXT_CHECK.lock() {
        *g = None;
    }
}

/// "https://www.Twitch.tv/Fulano/" -> "twitch.tv/fulano"
pub fn norm_url(u: &str) -> String {
    let u = u.trim().to_lowercase();
    let u = u.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.").trim_start_matches("m.");
    u.trim_end_matches('/').to_string()
}

/// Nome do streamer pelo link (pra credito).
pub fn streamer_name(url: &str) -> String {
    let n = norm_url(url);
    if let Some((_, r)) = n.split_once('@') {
        return r.split(['/', '?']).next().unwrap_or("").to_string();
    }
    n.split('/').nth(1).unwrap_or(&n).split('?').next().unwrap_or("").to_string()
}

/// true se a categoria desse streamer mudou desde a ultima conferencia (para logar so na troca).
fn note_category(key: &str, cat: &str) -> bool {
    static LAST: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(key.to_string(), cat.to_string()).as_deref() != Some(cat)
}

pub fn watching() -> Vec<String> {
    WATCHING.lock().ok().and_then(|g| g.as_ref().map(|s| s.iter().cloned().collect())).unwrap_or_default()
}

/// Ao vivo? Devolve a metadata do streamlink (categoria, titulo) se estiver; None = offline.
async fn live_meta(url: &str) -> Option<Value> {
    let o = tokio::process::Command::new(crate::live::streamlink_bin())
        .args(["--json", url])
        .tree_output_timeout(std::time::Duration::from_secs(60))
        .await
        .ok()?;
    let v: Value = serde_json::from_slice(&o.stdout).ok()?;
    v["streams"].as_object().is_some_and(|s| !s.is_empty()).then(|| v["metadata"].clone())
}

fn simple(s: &str) -> String {
    s.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A live serve pro canal? Reprise nunca; sem categoria (YouTube) passa; senao o jogo tem que bater.
/// (mesma regra do `categoria_bate` do scripts/live.py)
pub fn category_matches(meta: &Value, games: &[String]) -> (bool, String) {
    let title_text = meta["title"].as_str().unwrap_or("").to_lowercase();
    if title_text.starts_with("rerun") || title_text.contains("[rerun]") || title_text.starts_with("reprise") {
        return (false, "reprise".into());
    }
    let Some(cat) = meta["category"].as_str().filter(|c| !c.is_empty()) else { return (true, String::new()) };
    if games.is_empty() {
        return (true, cat.into());
    }
    let c = game_tokens(cat);
    let ok = games.iter().map(|g| game_tokens(g)).any(|g| !g.is_empty() && c.windows(g.len()).any(|w| w == g.as_slice()));
    (ok, cat.into())
}

/// Nome de jogo em palavras normalizadas: "gta" vira "grand theft auto" e numero romano vira arabico, para
/// "GTA VI", "GTA 6" e "Grand Theft Auto VI" serem o mesmo jogo. Antes a comparacao era por pedaco de texto nos
/// dois sentidos: "grand theft auto v" (GTA 5) esta dentro de "grand theft auto vi" e passava como GTA VI.
fn game_tokens(s: &str) -> Vec<String> {
    const ROMAN: [&str; 10] = ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"];
    let mut out: Vec<String> = Vec::new();
    for w in simple(s).split(' ').filter(|w| !w.is_empty()) {
        if w == "gta" {
            out.extend(["grand", "theft", "auto"].map(String::from));
        } else if let Some(i) = ROMAN.iter().position(|r| *r == w).filter(|_| !out.is_empty()) {
            // so depois de outra palavra ("auto vi"): "i"/"x" sozinhos podem ser palavra comum
            out.push((i + 1).to_string());
        } else {
            out.push(w.to_string());
        }
    }
    out
}

/// Liga o clipper nos streamers do canal que estao ao vivo (chamado a cada ciclo do piloto).
pub fn ensure_watchers(ch: &Channel) {
    let stop = stop_token();
    for url in &ch.streamers {
        let key = norm_url(url);
        let due = NEXT_CHECK.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&key).copied()));
        if due.is_some_and(|d| std::time::Instant::now() < d) {
            continue;
        }
        {
            let Ok(mut g) = WATCHING.lock() else { return };
            let set = g.get_or_insert_with(HashSet::new);
            if set.contains(&key) {
                continue;
            }
            if set.len() >= max_watch() {
                return;
            }
            set.insert(key.clone()); // reserva enquanto confere
        }
        let (url, slug, games) = (url.clone(), ch.slug.clone(), ch.games.clone());
        let stop = stop.clone();
        tokio::spawn(async move {
            let youtube = key.contains("youtube.com") || key.contains("youtu.be");
            // checar live no YouTube abre pagina + player (~2 requisicoes): entra no orcamento do IP
            let meta = if youtube && crate::yt_pace::reserve(2).await.is_err() { None } else { live_meta(&url).await };
            let switch_on = match &meta {
                None => false,
                Some(m) => {
                    let (ok, cat) = category_matches(m, &games);
                    if !ok && note_category(&key, &cat) {
                        // info uma vez por troca: no lancamento da pra ver por que um streamer ao vivo nao e cortado
                        tracing::info!("[{slug}] {url} ao vivo em \"{cat}\": fora dos jogos do canal, esperando trocar");
                    }
                    ok
                }
            };
            if switch_on {
                tracing::info!("[{slug}] {url} esta ao vivo: ligando o clipper");
                let o = crate::live::LiveOpts {
                    url: url.clone(),
                    minutes: 0.0,
                    threshold: 1.5,
                    min: 15.0,
                    max: 60.0,
                    lang: "auto".into(),
                    no_chat: false,
                    captions: true,
                    // durante a live so o corte bruto: whisper e 9:16 so nos finalistas (economiza CPU)
                    lazy: true,
                    games,
                    channel: slug.clone(),
                };
                tokio::select! {
                    _ = stop.cancelled() => tracing::info!("[{slug}] piloto desligado: parando o clipper de {url}"),
                    r = crate::live::run_live(o) => if let Err(e) = r {
                        tracing::warn!("[{slug}] clipper de {url}: {e:#}");
                    },
                }
                if let Ok(mut g) = ENDED.lock() {
                    g.get_or_insert_with(HashMap::new).insert(key.clone(), std::time::Instant::now());
                }
            } else if let Ok(mut g) = NEXT_CHECK.lock() {
                g.get_or_insert_with(HashMap::new).insert(key.clone(), std::time::Instant::now() + offline_backoff(&key));
            }
            if let Ok(mut g) = WATCHING.lock() {
                if let Some(s) = g.as_mut() {
                    s.remove(&key);
                }
            }
        });
    }
}

// ---------------------------------------------------------------- fila

#[derive(Debug, Clone)]
pub struct Candidate {
    pub json: PathBuf,
    pub mp4: PathBuf,
    pub info: Value,
    pub streamer: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Used {
    clip: String,
    id: String,
    result: String,
    at: String,
}

/// Arquivo antigo da lista de usados (antes do state.db). Migrado na primeira leitura e renomeado para
/// `.migrated` (nunca apagado).
fn legacy_used_path(ch: &Channel) -> PathBuf {
    ch.knowledge_dir().join("cortes.json")
}

/// Passa o `cortes.json` do canal para a tabela `clip_uses`, uma vez.
fn migrate_used(ch: &Channel, db: &rusqlite::Connection) -> Result<()> {
    let p = legacy_used_path(ch);
    if !p.exists() {
        return Ok(());
    }
    let old: Vec<Used> = crate::config::load_json(&p);
    let tx = db.unchecked_transaction()?;
    for u in &old {
        tx.execute(
            "INSERT INTO clip_uses (channel, clip, id, result, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![ch.slug, u.clip, u.id, u.result, u.at],
        )?;
    }
    tx.commit()?;
    std::fs::rename(&p, p.with_extension("json.migrated"))?;
    tracing::info!("[{}] {} cortes usados migrados do cortes.json para o state.db", ch.slug, old.len());
    Ok(())
}

/// Cortes que o canal ja usou (postado, barrado, recusado), da ultima semana.
fn load_used(ch: &Channel) -> Vec<Used> {
    let run = || -> Result<Vec<Used>> {
        let db = crate::state::side_db()?;
        migrate_used(ch, &db)?;
        let mut st = db.prepare("SELECT clip, id, result, at FROM clip_uses WHERE channel = ?1 AND at >= ?2 ORDER BY at")?;
        let rows = st.query_map(rusqlite::params![ch.slug, crate::state::days_ago(7)], |r| {
            Ok(Used { clip: r.get(0)?, id: r.get(1)?, result: r.get(2)?, at: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    run().unwrap_or_else(|e| {
        tracing::warn!("[{}] nao li os cortes usados: {e:#}", ch.slug);
        Vec::new()
    })
}

fn mark_used(ch: &Channel, clip: &Path, id: &str, result: &str) -> Result<()> {
    let db = crate::state::side_db()?;
    migrate_used(ch, &db)?;
    // so importa o que ainda pode ser candidato (MAX_AGE_H pela hora de criacao); o resto so crescia
    db.execute("DELETE FROM clip_uses WHERE at < ?1", rusqlite::params![crate::state::days_ago(7)])?;
    db.execute(
        "INSERT INTO clip_uses (channel, clip, id, result, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![ch.slug, rel(clip), id, result, crate::state::now()],
    )?;
    Ok(())
}

/// Caminhos (relativos a raiz) dos cortes que o canal ja usou na ultima semana.
pub fn used_clip_paths(ch: &Channel) -> Vec<String> {
    load_used(ch).into_iter().map(|u| u.clip).collect()
}

/// Cortes ja usados por QUALQUER canal de cortes: o mesmo corte nunca sai em dois canais.
fn used_everywhere(ch: &Channel) -> HashSet<String> {
    let mut chans: Vec<Channel> = crate::config::load_channels().unwrap_or_default().into_iter().filter(|c| c.is_clips()).collect();
    if !chans.iter().any(|c| c.slug == ch.slug) {
        chans.push(ch.clone());
    }
    chans.iter().flat_map(load_used).map(|u| u.clip).collect()
}

fn rel(p: &Path) -> String {
    p.strip_prefix(crate::config::root()).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// Cortes prontos (9:16) dos streamers do canal, ainda nao usados e recentes; melhor nota primeiro.
pub fn candidates(ch: &Channel) -> Vec<Candidate> {
    let mine: Vec<String> = ch.streamers.iter().map(|u| norm_url(u)).collect();
    let used = used_everywhere(ch);
    let live = crate::config::root().join("out").join("live");
    let mut out = Vec::new();
    let Ok(sessions) = std::fs::read_dir(&live) else { return out };
    for s in sessions.flatten() {
        let Ok(clips) = std::fs::read_dir(s.path().join("clips")) else { continue };
        for c in clips.flatten() {
            let p = c.path();
            if p.extension().is_none_or(|e| e != "json") || used.contains(&rel(&p)) {
                continue;
            }
            let mtime_age_h = c
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs_f64() / 3600.0)
                .unwrap_or(f64::MAX);
            // o mtime so pode ser mais novo que a criacao (o json e reescrito no acabamento): se ja passou, nem le
            if mtime_age_h > MAX_AGE_H {
                continue;
            }
            let Ok(info) = serde_json::from_str::<Value>(&std::fs::read_to_string(&p).unwrap_or_default()) else { continue };
            // idade de verdade = hora do corte; o mtime renova a cada acabamento e finalista nao escolhido
            // ganhava mais 48 h na fila
            if created_age_h(&info).is_some_and(|h| h > MAX_AGE_H) {
                continue;
            }
            if !mine.contains(&norm_url(info["url"].as_str().unwrap_or(""))) {
                continue;
            }
            // pronto (9:16) ou bruto (--lazy: acabamento so se for escolhido)
            let vert = info["files"]["vertical"].as_str().map(|v| p.with_file_name(v)).filter(|f| f.exists());
            let raw = info["files"]["raw"].as_str().map(|v| p.with_file_name(v)).filter(|f| f.exists());
            let Some(mp4) = vert.or(raw) else { continue };
            out.push(Candidate {
                streamer: info["channel"].as_str().map(String::from).unwrap_or_else(|| streamer_name(info["url"].as_str().unwrap_or(""))),
                score: info["score"].as_f64().unwrap_or(0.0),
                json: p,
                mp4,
                info,
            });
        }
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Um corte gravado pelo clipper (tela "Cortes" do app), de qualquer live.
#[derive(Debug, Clone, Serialize)]
pub struct ClipEntry {
    /// caminho do .json relativo a raiz (chave unica)
    pub key: String,
    /// mp4 absoluto (9:16 se ja tiver acabamento, senao o bruto)
    pub mp4: Option<String>,
    pub vertical: bool,
    pub platform: String,
    pub streamer: String,
    pub url: String,
    pub score: f64,
    pub duration: f64,
    pub created: String,
    pub category: Option<String>,
    pub chat: Vec<String>,
    pub transcript: String,
    /// canal de cortes que ja usou + resultado (ex.: "meu-canal: publicado")
    pub used: Option<String>,
}

/// Todos os cortes em out/live, mais novo primeiro (no maximo `limit`).
pub fn all_clips(limit: usize) -> Vec<ClipEntry> {
    let mut used: HashMap<String, String> = HashMap::new();
    for ch in crate::config::load_channels().unwrap_or_default().into_iter().filter(|c| c.is_clips()) {
        for u in load_used(&ch) {
            used.insert(u.clip, format!("{}: {}", ch.slug, u.result));
        }
    }
    let mut out = Vec::new();
    let Ok(sessions) = std::fs::read_dir(crate::config::root().join("out").join("live")) else { return out };
    for s in sessions.flatten() {
        let Ok(clips) = std::fs::read_dir(s.path().join("clips")) else { continue };
        for c in clips.flatten() {
            let p = c.path();
            if p.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(info) = serde_json::from_str::<Value>(&std::fs::read_to_string(&p).unwrap_or_default()) else { continue };
            let file = |k: &str| info["files"][k].as_str().map(|v| p.with_file_name(v)).filter(|f| f.exists());
            let vert = file("vertical");
            let vertical = vert.is_some();
            let mp4 = vert.or_else(|| file("raw")).map(|f| f.to_string_lossy().into_owned());
            let url = info["url"].as_str().unwrap_or("").to_string();
            let key = rel(&p);
            out.push(ClipEntry {
                used: used.get(&key).cloned(),
                key,
                mp4,
                vertical,
                platform: info["platform"].as_str().unwrap_or("").into(),
                streamer: info["channel"].as_str().map(String::from).unwrap_or_else(|| streamer_name(&url)),
                url,
                score: info["score"].as_f64().filter(|f| f.is_finite()).unwrap_or(0.0),
                duration: info["duration"].as_f64().filter(|f| f.is_finite()).unwrap_or(0.0),
                created: info["created"].as_str().unwrap_or("").into(),
                category: info["category"].as_str().map(String::from),
                chat: info["chat"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).take(4).collect()).unwrap_or_default(),
                transcript: info["transcript"].as_str().unwrap_or("").chars().take(300).collect(),
            });
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created));
    out.truncate(limit);
    out
}

/// Apaga um corte gravado (tela "Cortes"): o .json e os arquivos do mesmo corte (`clip_NN.*`, `clip_NN_*`).
/// `key` = caminho do .json relativo a raiz, como em `ClipEntry::key`. So dentro de out/live/<sessao>/clips.
/// Devolve quantos arquivos saíram.
pub fn delete_clip(key: &str) -> Result<usize> {
    let base = crate::config::root().join("out").join("live").canonicalize().context("sem out/live")?;
    let json = crate::config::root().join(key).canonicalize().context("corte nao encontrado")?;
    let dir = json.parent().context("corte sem pasta")?;
    let ok = json.extension().is_some_and(|e| e == "json")
        && json.starts_with(&base)
        && dir.file_name().is_some_and(|n| n == "clips")
        && dir.parent().and_then(|p| p.parent()).is_some_and(|p| p == base);
    anyhow::ensure!(ok, "isso nao e um corte de live");
    let stem = json.file_stem().map(|s| s.to_string_lossy().into_owned()).context("nome invalido")?;
    let mut n = 0;
    for f in std::fs::read_dir(dir)?.flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        // clip_1.* e clip_1_* (9x16, srt), nunca clip_10
        if name.starts_with(&format!("{stem}.")) || name.starts_with(&format!("{stem}_")) {
            std::fs::remove_file(f.path()).with_context(|| format!("nao consegui apagar {name}"))?;
            n += 1;
        }
    }
    tracing::info!("corte apagado pelo app: {key} ({n} arquivos)");
    Ok(n)
}

/// Horas desde `created` (hora local gravada pelo live.py, "%Y-%m-%dT%H:%M:%S").
fn created_age_h(info: &Value) -> Option<f64> {
    use chrono::TimeZone;
    let t = chrono::NaiveDateTime::parse_from_str(info["created"].as_str()?, "%Y-%m-%dT%H:%M:%S").ok()?;
    let t = chrono::Local.from_local_datetime(&t).earliest()?;
    Some((chrono::Local::now() - t).num_seconds() as f64 / 3600.0)
}

// ---------------------------------------------------------------- barreira tecnica

#[derive(Debug, Clone, Default, Serialize)]
pub struct Tech {
    pub duration: f64,
    pub lufs: Option<f64>,
    pub true_peak: Option<f64>,
    pub frozen_s: f64,
    pub black_start_s: f64,
    /// motivos de reprovacao (vazio = passou)
    pub fail: Vec<String>,
    /// avisos que nao reprovam (vao pro Claude)
    pub warn: Vec<String>,
}

fn num_after(s: &str, key: &str) -> Option<f64> {
    let i = s.find(key)? + key.len();
    let t: String = s[i..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    t.parse().ok()
}

/// Regras medidas (nada de modelo): duracao, tela congelada, comeco preto, audio.
pub fn judge_tech(duration: f64, lufs: Option<f64>, tp: Option<f64>, frozen_s: f64, black_start_s: f64) -> Tech {
    let mut t = Tech { duration, lufs, true_peak: tp, frozen_s, black_start_s, ..Default::default() };
    if !(12.0..=75.0).contains(&duration) {
        t.fail.push(format!("duração {duration:.0}s fora de 12-75 s"));
    }
    if duration > 0.0 && frozen_s / duration >= 0.25 {
        t.fail.push(format!("tela congelada {frozen_s:.0}s ({:.0}% do corte)", frozen_s / duration * 100.0));
    } else if frozen_s >= 2.0 {
        t.warn.push(format!("{frozen_s:.1}s de tela parada"));
    }
    if black_start_s >= 1.0 {
        t.fail.push(format!("começa com {black_start_s:.1}s de tela preta"));
    }
    match lufs {
        Some(l) if l < -32.0 => t.fail.push(format!("áudio quase mudo ({l:.0} LUFS)")),
        Some(l) if l < -24.0 => t.warn.push(format!("áudio baixo ({l:.0} LUFS)")),
        _ => {}
    }
    if let Some(p) = tp {
        if p > 2.0 {
            t.fail.push(format!("áudio estourado (pico {p:+.1} dBFS)"));
        } else if p > 0.0 {
            t.warn.push(format!("pico de áudio no limite ({p:+.1} dBFS)"));
        }
    }
    t
}

/// `tech_check` guardado no json do corte (medidas cruas; as regras rodam de novo a cada vez). Finalista bom
/// nao escolhido volta na proxima rodada e era medido de novo (decode inteiro).
async fn tech_check_cached(c: &mut Candidate) -> Result<Tech> {
    let file = c.mp4.file_name().unwrap_or_default().to_string_lossy().to_string();
    let m = &c.info["tech_measured"];
    if m["file"].as_str() == Some(file.as_str()) {
        if let (Some(d), Some(fz), Some(bs)) = (m["duration"].as_f64(), m["frozen_s"].as_f64(), m["black_start_s"].as_f64()) {
            return Ok(judge_tech(d, m["lufs"].as_f64(), m["true_peak"].as_f64(), fz, bs));
        }
    }
    let t = tech_check(&c.mp4).await?;
    c.info["tech_measured"] = serde_json::json!({
        "file": file, "duration": t.duration, "lufs": t.lufs, "true_peak": t.true_peak,
        "frozen_s": t.frozen_s, "black_start_s": t.black_start_s,
    });
    let tmp = c.json.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(&c.info)?).is_ok() {
        let _ = std::fs::rename(&tmp, &c.json);
    }
    Ok(t)
}

pub async fn tech_check(mp4: &Path) -> Result<Tech> {
    let probe = crate::media::probe(mp4).await?;
    let path = mp4.to_string_lossy().to_string();
    let ff = crate::media::ffmpeg_bin();
    let (_, err) = crate::media::run(
        &ff,
        &[
            "-v", "info", "-nostats", "-i", &path,
            "-vf", "scale=320:-2,freezedetect=n=-55dB:d=1.5,blackdetect=d=0.4:pix_th=0.10",
            "-af", "ebur128=peak=true",
            "-f", "null", "-",
        ],
    )
    .await?;
    let frozen: f64 = err.lines().filter_map(|l| num_after(l, "freeze_duration:")).sum();
    let black_start = err
        .lines()
        .filter(|l| l.contains("black_start:"))
        .filter_map(|l| Some((num_after(l, "black_start:")?, num_after(l, "black_end:")?)))
        .find(|(s, _)| *s < 0.2)
        .map(|(_, e)| e)
        .unwrap_or(0.0);
    let summary = err.rsplit("Summary:").next().unwrap_or("");
    let lufs = summary.lines().find_map(|l| num_after(l, "I:"));
    let tp = summary.lines().find_map(|l| num_after(l, "Peak:"));
    Ok(judge_tech(probe.duration, lufs, tp, frozen, black_start))
}

// ---------------------------------------------------------------- acabamento sob demanda

/// whisper (`speech`) e/ou 9:16 com legenda (`vertical`) de um corte bruto; recarrega o json.
async fn finish(c: &mut Candidate, vertical: bool) -> Result<()> {
    let has_speech = c.info["speech"].as_array().is_some_and(|a| !a.is_empty()) || c.info["speech_done"].as_bool() == Some(true);
    let is_vert = c.mp4.to_string_lossy().ends_with("_9x16.mp4");
    if has_speech && (!vertical || is_vert) {
        return Ok(());
    }
    let mut args = vec!["-I".to_string(), "-X".into(), "utf8".into(), "scripts/clip_finish.py".into(), c.json.to_string_lossy().to_string(), "--speech".into()];
    if vertical {
        args.extend(["--vertical".into(), "--captions".into()]);
    }
    let out = tokio::process::Command::new(crate::recreate::venv_bin("python"))
        .args(&args)
        .env("PYTHONUTF8", "1")
        .env("TKD_FFMPEG", crate::media::ffmpeg_bin())
        .current_dir(crate::config::root())
        .tree_output()
        .await
        .context("scripts/clip_finish.py")?;
    anyhow::ensure!(
        out.status.success(),
        "acabamento do corte falhou: {}",
        String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")
    );
    c.info = serde_json::from_str(&std::fs::read_to_string(&c.json)?)?;
    if let Some(v) = c.info["files"]["vertical"].as_str() {
        let f = c.json.with_file_name(v);
        if f.exists() {
            c.mp4 = f;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- Claude

/// Nota do Claude para cada corte comparado.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipEval {
    pub i: usize,
    /// e do tema do canal? (fora do tema = nunca posta)
    #[serde(default)]
    #[serde(rename = "no_tema")]
    pub on_topic: bool,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    #[serde(rename = "motivo")]
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipPick {
    #[serde(default)]
    #[serde(rename = "avaliacoes")]
    pub evaluations: Vec<ClipEval>,
    /// indice do escolhido; -1 = nenhum serve
    #[serde(rename = "escolhido")]
    pub chosen: i64,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub best_part: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
    /// recorte sugerido do escolhido (s, dentro do corte)
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

impl ClipPick {
    pub fn eval_of(&self, i: usize) -> Option<&ClipEval> {
        self.evaluations.iter().find(|a| a.i == i)
    }
}

fn speech_text(info: &Value) -> String {
    let lines: Vec<String> = info["speech"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let t = l["text"].as_str()?;
                    Some(format!("[{:.1}s] {t}", l["start"].as_f64().unwrap_or(0.0)))
                })
                .collect()
        })
        .unwrap_or_default();
    if lines.is_empty() {
        info["transcript"].as_str().unwrap_or("(sem fala transcrita)").to_string()
    } else {
        lines.join("\n")
    }
}

fn chat_text(info: &Value) -> String {
    let msgs: Vec<String> = info["chat"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).take(150).collect())
        .unwrap_or_default();
    if msgs.is_empty() { "(sem chat)".into() } else { msgs.join("\n") }
}

fn clip_block(i: usize, c: &Candidate, tech: &Tech, frames: &[PathBuf]) -> String {
    format!(
        "### Corte {i}\nStreamer: {} ({}) | categoria da live: {} | duracao {:.1} s | nota do momento {:.2} | sinais {}\nAvisos tecnicos: {}\n\nFala:\n{}\n\nChat:\n{}\n\nQuadros (Read):\n{}\n",
        c.streamer,
        c.info["platform"].as_str().unwrap_or(""),
        c.info["category"].as_str().filter(|s| !s.is_empty()).unwrap_or("desconhecida"),
        tech.duration,
        c.score,
        c.info["breakdown"],
        if tech.warn.is_empty() { "nenhum".into() } else { tech.warn.join("; ") },
        speech_text(&c.info),
        chat_text(&c.info),
        frames.iter().map(|p| format!("- {}", p.display())).collect::<Vec<_>>().join("\n"),
    )
}

async fn ask_claude(ch: &Channel, blocks: &[String]) -> Result<ClipPick> {
    let mut vars = HashMap::new();
    vars.insert("slug", ch.slug.clone());
    vars.insert("niche", ch.niche.clone());
    vars.insert("persona", ch.persona.clone());
    vars.insert("n", blocks.len().to_string());
    vars.insert("max_i", (blocks.len().saturating_sub(1)).to_string());
    vars.insert("clips", blocks.join("\n"));
    vars.insert("notes", ch.notes_text());
    vars.insert("hashtags", if ch.hashtags.is_empty() { "(nenhuma)".into() } else { ch.hashtags.join(" ") });
    let prompt = agent::render_template("review_clips.md", &vars)?;
    agent::ask_json(ch, "review_clips", &prompt, &["Read"], 420, 2).await
}

// ---------------------------------------------------------------- montagem

fn reviews_dir() -> PathBuf {
    crate::config::root().join("out").join("reviews")
}

/// Veredito do Claude guardado pra etapa de review (nao chama de novo).
pub fn stored_verdict(id: &str) -> Option<Verdict> {
    // `<id>.cortes.json` = nome antigo (antes do rename para ingles), ainda lido
    let text = [format!("{id}.clip-pick.json"), format!("{id}.cortes.json")]
        .iter()
        .find_map(|n| std::fs::read_to_string(reviews_dir().join(n)).ok())?;
    serde_json::from_str(&text).ok()
}

/// Trecho final [s, e] do corte (sugestao do Claude, com pelo menos 5 s).
fn trim_range(dur: f64, start: Option<f64>, end: Option<f64>) -> (f64, f64) {
    // sem clamp(min, max): com corte < 5 s o min passava do max e o f64::clamp da panic
    let dur = if dur.is_finite() { dur.max(0.0) } else { 0.0 };
    let s = start.filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0).min((dur - 5.0).max(0.0));
    let e = end.filter(|v| v.is_finite()).unwrap_or(dur).min(dur).max((s + 5.0).min(dur));
    (s, e)
}

/// 9:16 com legenda direto do bruto, ja no trecho final e no arquivo do video: um x264 so desde o bruto
/// (antes: 9:16 do corte inteiro e outro x264 + AAC so para recortar).
async fn render_final(c: &Candidate, dst: &Path, s: f64, e: f64) -> Result<()> {
    let out = tokio::process::Command::new(crate::recreate::venv_bin("python"))
        .args(["-I", "-X", "utf8", "scripts/clip_finish.py"])
        .arg(&c.json)
        .args(["--speech", "--vertical", "--captions", "--ss", &format!("{s:.2}"), "--to", &format!("{e:.2}"), "--out"])
        .arg(dst)
        .env("PYTHONUTF8", "1")
        .env("TKD_FFMPEG", crate::media::ffmpeg_bin())
        .current_dir(crate::config::root())
        .tree_output()
        .await
        .context("scripts/clip_finish.py")?;
    anyhow::ensure!(
        out.status.success() && dst.exists(),
        "9:16 do corte falhou: {}",
        String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")
    );
    Ok(())
}

/// Recorta o 9:16 no trecho sugerido (re-encode na CPU; sem recorte = copia).
async fn trim(src: &Path, dst: &Path, dur: f64, start: Option<f64>, end: Option<f64>) -> Result<f64> {
    let (s, e) = trim_range(dur, start, end);
    if s < 0.5 && e > dur - 0.5 {
        crate::media::link_or_copy(src, dst)?;
        return Ok(dur);
    }
    let st = tokio::process::Command::new(crate::media::ffmpeg_bin())
        .args(["-y", "-v", "error", "-ss", &format!("{s:.2}"), "-to", &format!("{e:.2}"), "-i"])
        .arg(src)
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .arg(dst)
        .tree_status()
        .await
        .context("ffmpeg (recorte)")?;
    anyhow::ensure!(st.success() && dst.exists(), "recorte do corte falhou");
    Ok(e - s)
}

/// Legenda com credito ao streamer (obrigatorio em corte de live).
pub fn credited(caption: &str, streamer: &str, platform: &str) -> String {
    let c = caption.trim();
    let plat = match platform {
        "twitch" => " na Twitch",
        "kick" => " na Kick",
        "youtube" => " no YouTube",
        _ => "",
    };
    format!("{c} | corte da live de {streamer}{plat}").trim_start_matches(" | ").to_string()
}

fn clip_spec(ch: &Channel, id: &str, title: &str, dur: f64, caption: &str, hashtags: &[String], clip: &str) -> Result<Spec> {
    let v = serde_json::json!({
        "id": id, "channel": ch.slug, "title": title, "durationS": dur, "aspect": "9:16", "fps": 30.0,
        "palette": { "bg": "#000000", "fg": "#ffffff", "accent": "#9146ff" },
        "typography": { "family": "Inter", "weight": 800, "case": "upper", "sizeVh": 5.0 },
        "treatment": { "grain": 0.0, "vignette": 0.0, "zoom": "none" },
        "scenes": [{ "startS": 0.0, "durationS": dur, "kind": "media", "text": "", "media": format!("live/{clip}") }],
        "caption": caption, "hashtags": hashtags,
    });
    serde_json::from_value(v).context("montando a spec do corte")
}

/// Pega o melhor corte da fila, passa na barreira tecnica e no Claude, e registra como video.
pub async fn make_clip(ch: &Channel, store: &Store) -> Result<Spec> {
    let span = crate::events::step(&ch.slug, "ideate");
    span.action("Escolhendo o corte da live", "ffmpeg");
    let r = make_inner(ch, store, &span).await;
    span.finish(r, |s| s.title.clone())
}

async fn make_inner(ch: &Channel, store: &Store, span: &crate::events::Span) -> Result<Spec> {
    anyhow::ensure!(!ch.streamers.is_empty(), "[{}] canal de cortes sem streamers: adicione em Editar canal", ch.slug);
    let cands = candidates(ch);
    anyhow::ensure!(
        !cands.is_empty(),
        "[{}] nenhum corte novo das últimas {MAX_AGE_H:.0} h: os streamers não fizeram live (ou o clipper não pegou nada forte)",
        ch.slug
    );
    let id = format!("{}-{}", ch.slug, chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    // 1) barreira tecnica (gratis) ate juntar POOL cortes bons
    let mut pool: Vec<(Candidate, Tech)> = Vec::new();
    for mut c in cands.into_iter().take(MAX_TRIES) {
        if pool.len() >= POOL {
            break;
        }
        span.action(&format!("Conferindo corte de {} (nota do momento {:.1})", c.streamer, c.score), "ffmpeg");
        match tech_check_cached(&mut c).await {
            Ok(t) if t.fail.is_empty() => pool.push((c, t)),
            Ok(t) => {
                tracing::info!("[{}] corte {} barrado: {}", ch.slug, rel(&c.mp4), t.fail.join("; "));
                mark_used(ch, &c.json, "", &format!("barrado: {}", t.fail.join("; ")))?;
            }
            Err(e) => {
                tracing::warn!("[{}] {}: {e:#}", ch.slug, c.mp4.display());
                mark_used(ch, &c.json, "", "erro na medição")?;
            }
        }
    }
    anyhow::ensure!(!pool.is_empty(), "[{}] os melhores cortes falharam na barreira técnica", ch.slug);

    // 2) Claude compara todos juntos: tema obrigatorio + escolhe o melhor
    span.action(&format!("Claude comparando {} cortes", pool.len()), "claude");
    let frames_dir = crate::config::root().join("out").join("frames").join(&id);
    let _frames_cleanup = crate::proc::RemoveOnDrop(frames_dir.clone());
    // finalistas brutos ganham a fala agora (o Claude le a transcricao)
    for (c, _) in pool.iter_mut() {
        span.action(&format!("Transcrevendo o corte de {}", c.streamer), "python");
        if let Err(e) = finish(c, false).await {
            tracing::warn!("[{}] {}: {e:#}", ch.slug, rel(&c.json));
        }
    }
    let mut blocks = Vec::new();
    for (i, (c, t)) in pool.iter().enumerate() {
        let fr = crate::media::frames(&c.mp4, &frames_dir.join(format!("c{i}")), FRAMES_PER_CLIP).await?;
        blocks.push(clip_block(i, c, t, &fr));
    }
    let out = ask_claude(ch, &blocks).await?;

    // fora do tema e nota baixa saem da fila (nao voltam a custar Claude); os bons nao escolhidos ficam
    let chosen = usize::try_from(out.chosen).ok().filter(|&i| i < pool.len());
    for (i, (c, _)) in pool.iter().enumerate() {
        if Some(i) == chosen {
            continue;
        }
        match out.eval_of(i) {
            Some(a) if !a.on_topic => mark_used(ch, &c.json, "", &format!("fora do tema: {}", a.rationale))?,
            Some(a) if a.score < crate::review::MIN_SCORE => mark_used(ch, &c.json, "", &format!("nota {:.1}: {}", a.score, a.rationale))?,
            None => mark_used(ch, &c.json, "", "sem avaliação")?,
            _ => {}
        }
    }
    let Some(i) = chosen else {
        anyhow::bail!("[{}] nenhum dos {} cortes serve (fora do tema ou fraco)", ch.slug, pool.len());
    };
    let a = out.eval_of(i).cloned().unwrap_or(ClipEval { i, on_topic: true, score: 0.0, rationale: String::new() });
    let keep = a.on_topic && a.score >= crate::review::MIN_SCORE;
    if !keep {
        // a review ia reprovar logo depois: nao gasta o encode do 9:16
        let rationale = if a.on_topic { format!("nota {:.1}: {}", a.score, a.rationale) } else { format!("fora do tema: {}", a.rationale) };
        mark_used(ch, &pool[i].0.json, "", &format!("escolhido mas reprovado ({rationale})"))?;
        anyhow::bail!("[{}] o melhor dos {} cortes nao passa ({rationale})", ch.slug, pool.len());
    }
    // so o escolhido ganha o 9:16 com legenda
    span.action("Montando o 9:16 do escolhido", "ffmpeg");
    let mp4 = crate::render::mp4_path(&id);
    std::fs::create_dir_all(mp4.parent().unwrap_or(Path::new(".")))?;
    let is_vert = pool[i].0.mp4.to_string_lossy().ends_with("_9x16.mp4");
    let dur = if is_vert {
        // corte antigo (sem --lazy) ja tem o 9:16 inteiro: so recorta
        let (c, tech) = &pool[i];
        let d = crate::media::probe(&c.mp4).await.map(|p| p.duration).unwrap_or(tech.duration);
        trim(&c.mp4, &mp4, d, out.start, out.end).await?
    } else {
        let (s, e) = trim_range(pool[i].1.duration, out.start, out.end);
        render_final(&pool[i].0, &mp4, s, e).await?;
        crate::media::probe(&mp4).await.map(|p| p.duration).unwrap_or(e - s)
    };
    let (c, tech) = &pool[i];
    let platform = c.info["platform"].as_str().unwrap_or("");
    let caption = credited(&out.caption, &c.streamer, platform);
    let title = if out.title.trim().is_empty() { c.streamer.clone() } else { out.title.trim().to_string() };
    let clip_name = c.mp4.file_name().unwrap_or_default().to_string_lossy().to_string();
    let spec = clip_spec(ch, &id, &title, dur, &caption, &out.hashtags, &clip_name)?;

    std::fs::create_dir_all(crate::ideate::specs_dir())?;
    std::fs::write(crate::ideate::spec_path(&id), serde_json::to_string_pretty(&spec)?)?;
    store.record_video(&id, &ch.slug, &serde_json::to_string(&spec)?, &mp4.to_string_lossy())?;
    let verdict = Verdict {
        score: a.score,
        keep,
        problems: out.problems.clone(),
        best_part: out.best_part.clone(),
    };
    std::fs::create_dir_all(reviews_dir())?;
    std::fs::write(reviews_dir().join(format!("{id}.clip-pick.json")), serde_json::to_string_pretty(&verdict)?)?;
    std::fs::write(
        reviews_dir().join(format!("{id}.clip-pick-full.json")),
        serde_json::to_string_pretty(&serde_json::json!({
            "clip": rel(&c.json),
            "comparados": pool.iter().map(|(c, _)| rel(&c.json)).collect::<Vec<_>>(),
            "tech": tech, "claude": out,
        }))?,
    )?;
    mark_used(ch, &c.json, &id, &format!("escolhido entre {}, nota {:.1}", pool.len(), a.score))?;
    store.log(&ch.slug, "ideate", "ok", &format!("corte de {} ({})", c.streamer, title))?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O formato da resposta do Claude (agents/review_clips.md) continua com as chaves em portugues.
    #[test]
    fn pick_reads_portuguese_keys() {
        let j = r#"{"avaliacoes":[{"i":1,"no_tema":true,"score":7.5,"motivo":"boa"}],"escolhido":1,"title":"T"}"#;
        let p: ClipPick = serde_json::from_str(j).unwrap();
        assert_eq!(p.chosen, 1);
        let e = p.eval_of(1).unwrap();
        assert!(e.on_topic);
        assert_eq!(e.rationale, "boa");
        let back = serde_json::to_value(&p).unwrap();
        assert!(back.get("avaliacoes").is_some() && back.get("escolhido").is_some());
    }

    #[test]
    fn age_from_creation() {
        let t = (chrono::Local::now() - chrono::Duration::hours(50)).format("%Y-%m-%dT%H:%M:%S").to_string();
        let h = created_age_h(&serde_json::json!({ "created": t })).unwrap();
        assert!((49.9..50.1).contains(&h));
        assert!(created_age_h(&serde_json::json!({})).is_none());
    }

    #[test]
    fn final_stretch() {
        assert_eq!(trim_range(40.0, Some(8.0), Some(28.0)), (8.0, 28.0));
        assert_eq!(trim_range(40.0, None, None), (0.0, 40.0));
        assert_eq!(trim_range(40.0, Some(38.0), Some(39.0)), (35.0, 40.0)); // pelo menos 5 s
    }

    #[test]
    fn urls_and_names() {
        assert_eq!(norm_url("https://www.Twitch.tv/Fulano/"), "twitch.tv/fulano");
        assert_eq!(streamer_name("https://kick.com/streamer_a"), "streamer_a");
        assert_eq!(streamer_name("https://www.youtube.com/@streamer_b/live"), "streamer_b");
    }

    #[test]
    fn tech_gate() {
        assert!(judge_tech(30.0, Some(-14.0), Some(-1.0), 0.0, 0.0).fail.is_empty());
        assert!(!judge_tech(8.0, Some(-14.0), Some(-1.0), 0.0, 0.0).fail.is_empty());
        assert!(!judge_tech(30.0, Some(-14.0), Some(3.5), 0.0, 0.0).fail.is_empty());
        assert!(!judge_tech(30.0, Some(-14.0), Some(-1.0), 10.0, 0.0).fail.is_empty());
        assert!(!judge_tech(30.0, Some(-40.0), Some(-1.0), 0.0, 0.0).fail.is_empty());
        assert!(!judge_tech(30.0, Some(-14.0), Some(-1.0), 0.0, 1.5).fail.is_empty());
        let w = judge_tech(30.0, Some(-26.0), Some(0.5), 3.0, 0.0);
        assert!(w.fail.is_empty() && w.warn.len() == 3);
    }

    #[test]
    fn crop_never_panics() {
        assert_eq!(trim_range(3.0, Some(1.0), Some(2.0)), (0.0, 3.0));
        assert_eq!(trim_range(30.0, Some(20.0), Some(10.0)), (20.0, 25.0));
        assert_eq!(trim_range(30.0, Some(-4.0), Some(99.0)), (0.0, 30.0));
        assert_eq!(trim_range(30.0, Some(f64::NAN), None), (0.0, 30.0));
        assert_eq!(trim_range(0.0, None, None), (0.0, 0.0));
    }

    #[test]
    fn live_category() {
        let gta = vec!["Grand Theft Auto VI".to_string()];
        let m = |c: &str, t: &str| serde_json::json!({"category": c, "title": t});
        assert!(category_matches(&m("Grand Theft Auto VI", "rp"), &gta).0);
        assert!(!category_matches(&m("Counter-Strike", "ao vivo"), &gta).0);
        assert!(!category_matches(&m("Grand Theft Auto VI", "RERUN: ontem"), &gta).0);
        assert!(category_matches(&serde_json::json!({}), &gta).0);
        assert!(category_matches(&m("Just Chatting", "oi"), &[]).0);
        // lancamento do GTA VI: GTA 5 nao pode passar; os nomes curtos da Kick/YouTube sim
        assert!(!category_matches(&m("Grand Theft Auto V", "rp"), &gta).0, "GTA 5 passava como GTA VI");
        assert!(!category_matches(&m("GTA V", "rp"), &gta).0);
        assert!(!category_matches(&m("Grand Theft Auto V (GTA)", "rp"), &gta).0, "nome da Kick");
        assert!(!category_matches(&m("Grand Theft Auto IV", "x"), &gta).0);
        assert!(!category_matches(&m("Auto", "x"), &gta).0, "pedaco do nome nao vale");
        for ok in ["GTA VI", "GTA 6", "gta-vi", "Grand Theft Auto 6", "GRAND THEFT AUTO VI", "Grand Theft Auto VI: Online", "Grand Theft Auto VI (GTA)"] {
            assert!(category_matches(&m(ok, "lancamento"), &gta).0, "{ok}");
        }
        assert!(category_matches(&m("Grand Theft Auto V", "rp"), &["GTA V".to_string()]).0);
    }

    #[test]
    fn credit_in_caption() {
        assert_eq!(credited("ele surtou", "streamer_a", "twitch"), "ele surtou | corte da live de streamer_a na Twitch");
        assert_eq!(credited("", "x", "kick"), "corte da live de x na Kick");
    }

    #[test]
    fn reads_ffmpeg() {
        assert_eq!(num_after("  I:         -14.2 LUFS", "I:"), Some(-14.2));
        assert_eq!(num_after("lavfi.freezedetect.freeze_duration: 2.5", "freeze_duration:"), Some(2.5));
        assert_eq!(num_after("black_start:0 black_end:1.2 black_duration:1.2", "black_end:"), Some(1.2));
    }
}
