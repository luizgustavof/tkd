//! Ritmo das chamadas ao YouTube (yt-dlp). Vale com ou sem os cookies da conta descartavel: o limite e por IP.
//!
//! Limite observado sem login (wiki do yt-dlp): ~300 videos / ~1000 requisicoes por hora por IP.
//! Orcamento persistido em disco (vale entre execucoes do `tkd` e e dividido com scripts/yt_pace.py):
//!   - cada chamada do yt-dlp reserva um peso ~ requisicoes que ela faz;
//!   - intervalo minimo entre chamadas;
//!   - bloqueio (429 / "not a bot") = pausa gravada; ate ela acabar nenhuma chamada sai (insistir piora).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// requisicoes por hora que nos permitimos. Era 600 ("o limite e ~1000"), mas em 17/11/2026 o YouTube barrou o IP
/// ("not a bot") marcando 60/600: o limite real deste IP e bem mais baixo. Igual ao scripts/yt_pace.py.
const HOURLY_BUDGET: u32 = 200;
const WINDOW: u64 = 3600;
/// segundos minimos entre duas chamadas do yt-dlp
const MIN_INTERVAL: u64 = 4;
/// pausa depois de um bloqueio do YouTube; cresce se o bloqueio volta logo apos a pausa
/// (1 h, 3 h, 6 h, 12 h): insistir num IP barrado so estica o bloqueio.
const PAUSES: [u64; 4] = [3600, 3 * 3600, 6 * 3600, 12 * 3600];
/// bloqueio ate 3 h depois do fim da pausa anterior conta como "seguido"
const STREAK_WINDOW: u64 = 3 * 3600;
/// espera maxima pelo orcamento antes de desistir
const MAX_WAIT: u64 = 15 * 60;

/// Peso aproximado (requisicoes HTTP ao YouTube) de cada tipo de chamada.
pub const WEIGHT_SEARCH: u32 = 6; // ytsearch3 + pagina de cada resultado
/// busca com --flat-playlist: so a pagina de resultados (titulo/duracao/id vem dela, nenhum video e aberto)
pub const WEIGHT_SEARCH_FLAT: u32 = 2;
pub const WEIGHT_DOWNLOAD: u32 = 4; // pagina + player + manifesto
pub const WEIGHT_META: u32 = 12; // info + ~10 paginas de comentarios (200)

/// Estado em disco. Os `alias` leem o arquivo antigo (out/.yt_ritmo.json, chaves em portugues).
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// (unix, peso) das chamadas da ultima hora
    #[serde(default, alias = "chamadas")]
    calls: Vec<(u64, u32)>,
    /// unix ate quando o YouTube esta bloqueando este IP
    #[serde(default, alias = "pausa_ate")]
    paused_until: u64,
    /// bloqueios seguidos (escala a pausa)
    #[serde(default, alias = "seguidos")]
    streak: u32,
}

/// (pausa em s, novo contador) para um bloqueio em `t`, dado o fim da pausa anterior.
fn next_pause(t: u64, prev_paused_until: u64, streak: u32) -> (u64, u32) {
    let n = if prev_paused_until > 0 && t <= prev_paused_until + STREAK_WINDOW { streak + 1 } else { 1 };
    (PAUSES[(n as usize - 1).min(PAUSES.len() - 1)], n)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `TKD_YT_PACE_FILE` troca o arquivo (experimentos de caos nao pausam o YouTube de verdade). O Python le a mesma.
fn state_file() -> PathBuf {
    match std::env::var_os("TKD_YT_PACE_FILE") {
        Some(p) => PathBuf::from(p),
        None => crate::config::root().join("out").join(".yt_pace.json"),
    }
}

/// arquivo de antes da troca de nome; lido so enquanto o novo nao existe (preserva uma pausa em andamento)
fn legacy_state_file() -> PathBuf {
    crate::config::root().join("out").join(".yt_ritmo.json")
}

fn read_state() -> State {
    let f = state_file();
    let f = if f.exists() { f } else { legacy_state_file() };
    std::fs::read_to_string(f)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_state(s: &State) {
    let p = state_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    // escrita atomica: o Python (scripts/yt_pace.py) le o mesmo arquivo
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string(s).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Trava entre processos (Rust e Python dividem o arquivo): `.lock` criado em modo exclusivo.
struct FileLock(PathBuf);

impl FileLock {
    fn acquire() -> FileLock {
        let p = PathBuf::from(format!("{}.lock", state_file().to_string_lossy()));
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let t0 = std::time::Instant::now();
        loop {
            if std::fs::OpenOptions::new().write(true).create_new(true).open(&p).is_ok() {
                return FileLock(p);
            }
            // trava esquecida por processo morto
            let stale = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .map(|m| m.elapsed().unwrap_or_default() > Duration::from_secs(10))
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_file(&p);
                continue;
            }
            if t0.elapsed() > Duration::from_secs(10) {
                return FileLock(PathBuf::new()); // segue sem trava em vez de travar tudo
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn clock(unix: u64) -> String {
    chrono::DateTime::from_timestamp(unix as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Espera a vez de chamar o YouTube e reserva `weight` requisicoes. Erro se o IP esta em pausa.
pub async fn reserve(weight: u32) -> Result<()> {
    loop {
        // a trava de arquivo espera com sleep de thread: fora do runtime para nao segurar um worker do tokio
        let lock = tokio::task::spawn_blocking(FileLock::acquire).await?;
        let mut s = read_state();
        let t = now();
        anyhow::ensure!(
            t >= s.paused_until,
            "YouTube bloqueou este IP; o tkd nao chama o YouTube ate {} para nao estender o bloqueio (so esperar)",
            clock(s.paused_until)
        );
        s.calls.retain(|(c, _)| t.saturating_sub(*c) < WINDOW);
        let used: u32 = s.calls.iter().map(|(_, w)| w).sum();
        let last = s.calls.iter().map(|(c, _)| *c).max().unwrap_or(0);

        let wait = if used + weight > HOURLY_BUDGET {
            // libera quando chamadas antigas suficientes saem da janela
            let mut needed = used + weight - HOURLY_BUDGET;
            let mut sorted = s.calls.clone();
            sorted.sort();
            let mut until = t;
            for (c, w) in sorted {
                until = c + WINDOW;
                if w >= needed {
                    break;
                }
                needed -= w;
            }
            let secs = until.saturating_sub(t) + 1;
            anyhow::ensure!(
                secs <= MAX_WAIT,
                "orcamento do YouTube da hora esgotado ({used}/{HOURLY_BUDGET}); libera as {}",
                clock(until)
            );
            tracing::info!("orcamento do YouTube: {used}/{HOURLY_BUDGET} na ultima hora; esperando {secs}s");
            secs
        } else if t < last + MIN_INTERVAL {
            last + MIN_INTERVAL - t
        } else {
            0
        };
        if wait > 0 {
            drop(lock);
            tokio::time::sleep(Duration::from_secs(wait.min(30))).await;
            continue;
        }
        s.calls.push((t, weight));
        write_state(&s);
        return Ok(());
    }
}

/// O erro do yt-dlp e bloqueio por excesso (429 / desafio anti-bot)?
pub fn is_block(err: &str) -> bool {
    err.contains("429") || err.contains("Too Many Requests") || err.contains("not a bot")
}

/// Grava a pausa: nenhuma chamada sai ate ela acabar.
pub fn mark_blocked() {
    let _lock = FileLock::acquire();
    let mut s = read_state();
    let t = now();
    let (pause, n) = next_pause(t, s.paused_until, s.streak);
    s.streak = n;
    s.paused_until = t + pause;
    write_state(&s);
    tracing::warn!("YouTube bloqueou o IP (429/anti-bot, {}o seguido); pausando chamadas ate {}", s.streak, clock(s.paused_until));
}

/// Opcoes de ritmo dentro do proprio yt-dlp (entre requisicoes e antes de cada download).
pub const SLEEP_ARGS: [&str; 6] = ["--sleep-requests", "1", "--sleep-interval", "5", "--max-sleep-interval", "10"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_grows_when_block_returns_soon() {
        assert_eq!(next_pause(10_000, 0, 0), (3600, 1));
        // bloqueou de novo 1 h depois do fim da pausa: 3 h
        assert_eq!(next_pause(10_000, 6_400, 1), (3 * 3600, 2));
        assert_eq!(next_pause(10_000, 6_400, 3), (12 * 3600, 4));
        assert_eq!(next_pause(10_000, 6_400, 9), (12 * 3600, 10));
        // muito depois: volta a 1 h
        assert_eq!(next_pause(100_000, 6_400, 3), (3600, 1));
    }

    #[test]
    fn reads_legacy_portuguese_keys() {
        let s: State = serde_json::from_str(r#"{"chamadas":[[1,6]],"pausa_ate":99,"seguidos":2}"#).unwrap();
        assert_eq!((s.calls.len(), s.paused_until, s.streak), (1, 99, 2));
    }
}
