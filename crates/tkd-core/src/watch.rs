use crate::{config, config::Channel, events, ideate, publish, render, review, scout, state::Store};
use anyhow::Result;
use chrono::{DateTime, Duration as CDuration, Local, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Resultado de um video do comeco ao fim.
#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    pub run_id: String,
    pub spec_id: String,
    pub score: f64,
    pub approved: bool,
    pub posted: bool,
    pub url: String,
}

/// Canais com pipeline rodando agora (nunca dois videos do mesmo canal ao mesmo tempo).
static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub fn busy_channels() -> Vec<String> {
    BUSY.lock().ok().and_then(|g| g.as_ref().map(|s| s.iter().cloned().collect())).unwrap_or_default()
}

struct BusyGuard(String, #[allow(dead_code)] crate::proc::ExclusiveFile);
impl BusyGuard {
    /// Canal livre neste processo E nos outros (app + CLI no mesmo canal faziam dois videos juntos).
    fn take(slug: &str) -> Option<Self> {
        let mut g = BUSY.lock().ok()?;
        let set = g.get_or_insert_with(HashSet::new);
        if set.contains(slug) {
            return None;
        }
        let lock = crate::proc::ExclusiveFile::try_acquire(&config::root().join("out").join(".busy").join(format!("{slug}.lock")))?;
        set.insert(slug.to_string());
        Some(BusyGuard(slug.to_string(), lock))
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = BUSY.lock() {
            if let Some(s) = g.as_mut() {
                s.remove(&self.0);
            }
        }
    }
}

/// Algo mudou desde o ultimo estudo? (referencia nova, medida ou reanalisada depois do style card)
fn study_stale(ch: &Channel) -> bool {
    let Some(card) = std::fs::metadata(ch.style_card()).and_then(|m| m.modified()).ok() else { return true };
    std::fs::read_dir(ch.ref_meta_dir())
        .map(|rd| {
            rd.flatten()
                .any(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t > card))
        })
        .unwrap_or(false)
}

/// "Estudar agora": busca os videos novos (de todos os perfis ou so de `handle`) e reescreve
/// o estilo. Sem nada novo desde o ultimo estudo, nao chama o Claude (nao gasta).
pub async fn study_now(ch: &Channel, store: &Store, handle: Option<&str>) -> Result<usize> {
    let _busy = BusyGuard::take(&ch.slug).ok_or_else(|| anyhow::anyhow!("[{}] o canal ja esta trabalhando agora", ch.slug))?;
    anyhow::ensure!(!ch.ref_handles().is_empty(), "adicione pelo menos um perfil pra estudar");
    let run_id = events::begin_run(&ch.slug);
    store.db().execute(
        "INSERT INTO runs (channel, step, status, detail, at, run_id) VALUES (?1,'run','start','estudo',?2,?3)",
        rusqlite::params![ch.slug, crate::state::now(), run_id],
    )?;
    let r = async {
        let n = match handle {
            Some(h) => scout::scout_one(ch, store, 8, h).await?,
            None => scout::scout(ch, store, 8).await?,
        };
        if study_stale(ch) {
            crate::study::study(ch, store, true).await?;
        } else {
            store.log(&ch.slug, "study", "ok", "nada novo desde o ultimo estudo: estilo mantido")?;
            tracing::info!("[{}] nada novo desde o ultimo estudo: nao chamou o Claude", ch.slug);
        }
        Ok::<usize, anyhow::Error>(n)
    }
    .await;
    let (status, detail) = match &r {
        Ok(n) => ("ok", format!("{n} referencias novas")),
        Err(e) => ("erro", format!("{e:#}")),
    };
    store.db().execute(
        "UPDATE runs SET status = ?2, detail = ?3, ended_at = ?4 WHERE run_id = ?1 AND step = 'run'",
        rusqlite::params![run_id, status, detail, crate::state::now()],
    )?;
    r
}

/// Pipeline de um video: ideate -> render -> review -> (publish se aprovado).
/// `dry_run` = "Fazer video de teste": vai ate o TikTok Studio mas nao publica.
pub async fn run_channel(ch: &Channel, store: &Store, theme: Option<&str>, dry_run: bool) -> Result<RunOutcome> {
    let _busy = BusyGuard::take(&ch.slug)
        .ok_or_else(|| anyhow::anyhow!("[{}] ja tem um video sendo feito agora", ch.slug))?;
    let run_id = events::begin_run(&ch.slug);
    let at = crate::state::now();
    store.db().execute(
        "INSERT INTO runs (channel, step, status, detail, at, run_id) VALUES (?1,'run','start',?2,?3,?4)",
        rusqlite::params![ch.slug, if dry_run { "teste" } else { "" }, at, run_id],
    )?;
    let r = pipeline(ch, store, theme, dry_run, &run_id).await;
    let (status, detail) = match &r {
        Ok(o) if o.posted => ("posted", o.url.clone()),
        Ok(o) if o.approved => ("ok", o.spec_id.clone()),
        Ok(o) => ("reprovado", format!("nota {}", o.score)),
        Err(e) => ("erro", format!("{e:#}")),
    };
    store.db().execute(
        "UPDATE runs SET status = ?2, detail = ?3, ended_at = ?4 WHERE run_id = ?1 AND step = 'run'",
        rusqlite::params![run_id, status, detail, crate::state::now()],
    )?;
    r
}

async fn pipeline(ch: &Channel, store: &Store, theme: Option<&str>, dry_run: bool, run_id: &str) -> Result<RunOutcome> {
    // canal de edit: footage + falas reais do personagem (o recreate ja entrega o mp4)
    let spec = if ch.is_edit() {
        crate::edit::make_edit(ch, store).await?
    } else if ch.is_clips() {
        // corte de live: o clipper ja cortou; aqui so escolhe, confere e legenda
        crate::clips::make_clip(ch, store).await?
    } else {
        let s = ideate::ideate(ch, store, theme).await?;
        render::render(ch, store, &s.id).await?;
        s
    };
    let verdict = review::review(ch, store, &spec.id).await?;
    let mut out = RunOutcome {
        run_id: run_id.to_string(),
        spec_id: spec.id.clone(),
        score: verdict.score,
        approved: verdict.keep && verdict.score >= review::MIN_SCORE,
        posted: false,
        url: String::new(),
    };
    if !out.approved {
        tracing::warn!("[{}] {} nao publicado (nota {})", ch.slug, spec.id, verdict.score);
        return Ok(out);
    }
    out.url = publish::publish(ch, store, &spec.id, dry_run).await?;
    out.posted = !dry_run;
    Ok(out)
}

// ---------------------------------------------------------------- agenda

/// Desvio deterministico (minutos) de um horario: estavel entre ticks, diferente por canal/dia.
pub fn slot_offset(slug: &str, date: NaiveDate, idx: usize, jitter_min: u32) -> i64 {
    if jitter_min == 0 {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (slug, date, idx).hash(&mut h);
    let span = 2 * jitter_min as u64 + 1;
    (h.finish() % span) as i64 - jitter_min as i64
}

/// Horarios do dia ja com o desvio aplicado.
pub fn slots_with_jitter(ch: &Channel, date: NaiveDate) -> Vec<DateTime<Local>> {
    ch.schedule
        .slots_on(date)
        .into_iter()
        .enumerate()
        .map(|(i, t)| t + CDuration::minutes(slot_offset(&ch.slug, date, i, ch.schedule.jitter_min)))
        .collect()
}

fn local_midnight_utc(now: DateTime<Local>) -> String {
    let d = now.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default();
    Local
        .from_local_datetime(&d)
        .earliest()
        .map(|t| t.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}

/// Proximo post previsto (nominal, sem desvio). None = nao posta (pausado, teste, sem horario).
pub fn next_post(ch: &Channel, store: &Store, now: DateTime<Local>) -> Option<DateTime<Local>> {
    if ch.paused || ch.is_test() {
        return None;
    }
    let done_today = store.posts_since(&ch.slug, &local_midnight_utc(now)).unwrap_or(0) as usize;
    let today = ch.schedule.slots_on(now.date_naive());
    // horarios de hoje ainda nao cumpridos (mesmo que ja passados por pouco, contam como "agora")
    if let Some(t) = today.iter().skip(done_today).find(|t| **t > now - CDuration::minutes(30)) {
        return Some(*t);
    }
    ch.schedule.next_slot(now.date_naive().and_hms_opt(23, 59, 59).and_then(|d| Local.from_local_datetime(&d).earliest()).unwrap_or(now))
}

/// Tentativas por (canal, dia, horario): reprovacao nao vira loop infinito de tokens. Ficam no state.db
/// (antes em memoria: zeravam no reboot e o mesmo horario ganhava mais tentativas).
const MAX_ATTEMPTS: u32 = 2;
/// Atraso maximo para ainda cumprir um horario perdido (app fechado, PC dormindo).
const MAX_LATE_H: i64 = 3;
/// Teto do intervalo minimo entre dois posts do mesmo canal.
const MAX_POST_GAP_MIN: i64 = 90;
/// Piso do intervalo minimo (com horarios muito juntos e o desvio aleatorio).
const MIN_POST_GAP_MIN: i64 = 10;

fn attempts(store: &Store, slug: &str, date: NaiveDate, idx: usize) -> u32 {
    store.slot_attempts(slug, &date.to_string(), idx).unwrap_or(0)
}

fn bump(store: &Store, slug: &str, date: NaiveDate, idx: usize, to: Option<u32>) {
    if let Err(e) = store.bump_slot_attempts(slug, &date.to_string(), idx, to) {
        tracing::warn!("[{slug}] nao gravei a tentativa do horario {}: {e:#}", idx + 1);
    }
}

/// Intervalo minimo entre posts do canal: metade do menor espaco entre horarios do dia, entre
/// `MIN_POST_GAP_MIN` e `MAX_POST_GAP_MIN`. Depois de um reboot dois horarios perdidos saiam com minutos de
/// diferenca; agora o segundo espera. Agenda apertada de proposito (09:30 e 10:00) continua valendo.
pub fn min_post_gap(ch: &Channel, date: NaiveDate) -> CDuration {
    let slots = slots_with_jitter(ch, date);
    let tightest = slots.windows(2).map(|w| (w[1] - w[0]).num_minutes().abs()).min().unwrap_or(MAX_POST_GAP_MIN * 2);
    CDuration::minutes((tightest / 2).clamp(MIN_POST_GAP_MIN, MAX_POST_GAP_MIN))
}

/// Ultimo post do canal (inclui upload em andamento), em hora local.
fn last_activity(store: &Store, slug: &str) -> Option<DateTime<Local>> {
    let t = store.last_activity_at(slug).ok().flatten()?;
    let n = chrono::NaiveDateTime::parse_from_str(&t, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(Utc.from_utc_datetime(&n).with_timezone(&Local))
}

/// Horario que deve rodar agora, se houver.
pub fn due_slot(ch: &Channel, store: &Store, now: DateTime<Local>) -> Option<usize> {
    if ch.paused || ch.is_test() {
        return None;
    }
    let date = now.date_naive();
    if last_activity(store, &ch.slug).is_some_and(|t| now - t < min_post_gap(ch, date)) {
        return None;
    }
    let done = store.posts_since(&ch.slug, &local_midnight_utc(now)).unwrap_or(0) as usize;
    slots_with_jitter(ch, date).into_iter().enumerate().skip(done).find_map(|(i, t)| {
        (t <= now && now - t < CDuration::hours(MAX_LATE_H) && attempts(store, &ch.slug, date, i) < MAX_ATTEMPTS).then_some(i)
    })
}

#[derive(Debug, Clone)]
pub struct WatchOpts {
    pub dry_run: bool,
    /// De quantas em quantas horas buscar referencias novas.
    pub scout_every_hours: i64,
    pub once: bool,
}

/// Manutencao do canal: referencias novas e estudo.
async fn maintain(ch: &Channel, store: &Store, every_h: i64) {
    let stale = store
        .last_step_at(&ch.slug, "scout")
        .ok()
        .flatten()
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| Utc::now() - t.with_timezone(&Utc) > CDuration::hours(every_h))
        .unwrap_or(true);
    if stale {
        match scout::scout(ch, store, 6).await {
            Ok(0) => tracing::info!("[{}] sem referencia nova", ch.slug),
            Ok(n) => {
                tracing::info!("[{}] {n} referencias novas", ch.slug);
                if let Err(e) = crate::study::study(ch, store, true).await {
                    tracing::error!("[{}] study: {e}", ch.slug);
                }
            }
            Err(e) => tracing::error!("[{}] scout: {e}", ch.slug),
        }
    }
    if !ch.style_card().exists() {
        if let Err(e) = crate::study::study(ch, store, false).await {
            tracing::error!("[{}] study: {e}", ch.slug);
        }
    }
}

async fn channel_tick(ch: Channel, store: Arc<Store>, o: WatchOpts) {
    if busy_channels().contains(&ch.slug) {
        return;
    }
    if ch.is_clips() {
        // liga o clipper em quem estiver ao vivo (o estudo de perfis nao se aplica a cortes)
        crate::clips::ensure_watchers(&ch);
    } else {
        maintain(&ch, &store, o.scout_every_hours).await;
    }
    let now = Local::now();
    let Some(idx) = due_slot(&ch, &store, now) else { return };
    // cortes com a fila vazia (live ainda nao comecou): nao gasta tentativa. Antes as 2 tentativas iam embora em
    // 2 minutos com "nenhum corte novo" e o horario morria, mesmo com corte saindo logo depois; agora o horario
    // espera corte aparecer dentro da janela de atraso (MAX_LATE_H)
    if ch.is_clips() && crate::clips::candidates(&ch).is_empty() {
        if warn_once(&format!("{}#fila", ch.slug), now.date_naive(), idx) {
            tracing::info!(
                "[{}] horario {} de hoje: fila de cortes vazia, esperando corte de live (ate {MAX_LATE_H} h)",
                ch.slug,
                idx + 1
            );
        }
        return;
    }
    // todas as redes com sessao caida: fazer o video so gastaria tokens e render para falhar no upload
    if !o.dry_run && !crate::publish::platforms(&ch).is_empty() {
        match crate::publish::live_platforms(&ch, &store) {
            Ok(v) if v.is_empty() => {
                if warn_once(&ch.slug, now.date_naive(), idx) {
                    tracing::warn!("[{}] sessao caida em todas as redes: conecte de novo no app (horario {} pulado)", ch.slug, idx + 1);
                    let _ = store.log(&ch.slug, "publish", "erro", "sessao caida em todas as redes: conecte de novo no app");
                }
                return;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("[{}] nao li o estado das sessoes: {e:#}", ch.slug),
        }
    }
    let date = now.date_naive();
    bump(&store, &ch.slug, date, idx, None);
    tracing::info!("[{}] horario {} de hoje: fazendo o video", ch.slug, idx + 1);
    match run_channel(&ch, &store, None, o.dry_run).await {
        Ok(r) if r.posted || (o.dry_run && r.approved) => bump(&store, &ch.slug, date, idx, Some(MAX_ATTEMPTS)),
        Ok(_) => {}
        Err(e) => tracing::error!("[{}] pipeline: {e:#}", ch.slug),
    }
}

/// Avisa uma vez por horario (o tick roda a cada minuto).
fn warn_once(slug: &str, date: NaiveDate, idx: usize) -> bool {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<(String, NaiveDate, usize)>>> = Mutex::new(None);
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert((slug.to_string(), date, idx))
}

/// Uma passada por todos os canais (cada canal na sua task; navegador e agentes tem fila propria).
pub async fn tick(store: Arc<Store>, o: &WatchOpts) -> Result<()> {
    let mut set = tokio::task::JoinSet::new();
    for ch in config::load_channels()? {
        if ch.paused {
            continue;
        }
        set.spawn(channel_tick(ch, store.clone(), o.clone()));
    }
    while set.join_next().await.is_some() {}
    Ok(())
}

/// Piloto automatico: confere a agenda a cada minuto ate ser cancelado.
pub async fn watch_loop(store: Arc<Store>, o: WatchOpts, cancel: CancellationToken) -> Result<()> {
    if !o.once {
        // lives num ritmo proprio: o ciclo espera o video mais lento de todos os canais (minutos), e uma live
        // que comeca nesse meio tempo perdia a abertura
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(chs) = config::load_channels() {
                    for ch in chs.iter().filter(|c| !c.paused && c.is_clips()) {
                        crate::clips::ensure_watchers(ch);
                    }
                }
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
            }
        });
    }
    loop {
        let mut t = tokio::spawn({
            let store = store.clone();
            let o = o.clone();
            async move { tick(store, &o).await }
        });
        tokio::select! {
            _ = cancel.cancelled() => { t.abort(); crate::clips::stop_watchers(); return Ok(()); }
            r = &mut t => if let Ok(Err(e)) = r { tracing::error!("ciclo falhou: {e}"); }
        }
        if o.once {
            return Ok(());
        }
        tokio::select! {
            _ = cancel.cancelled() => { crate::clips::stop_watchers(); return Ok(()); }
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
    }
}

/// Entrada da CLI (`tkd watch`).
pub async fn watch(store: Store, once: bool, every_hours: u64, dry_run: bool) -> Result<()> {
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        c2.cancel();
    });
    let o = WatchOpts { dry_run, scout_every_hours: every_hours.max(1) as i64, once };
    watch_loop(Arc::new(store), o, cancel).await
}

/// Pausa ou retoma o canal (grava `paused` no channel.toml).
pub fn set_paused(slug: &str, paused: bool) -> Result<Channel> {
    config::edit_channel(slug, |doc| {
        doc["paused"] = toml_edit::value(paused);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_stable_and_in_bounds() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let a = slot_offset("canal-a", d, 0, 10);
        assert_eq!(a, slot_offset("canal-a", d, 0, 10));
        for i in 0..50 {
            let o = slot_offset("canal-b", d + CDuration::days(i), 0, 10);
            assert!((-10..=10).contains(&o));
        }
        assert_eq!(slot_offset("x", d, 0, 0), 0);
    }

    fn test_channel(slug: &str, paused: bool) -> Channel {
        let mut ch: Channel = toml::from_str(&format!(
            "slug = \"{slug}\"
ref_handle = \"none\"
post_handle = \"\"
persona = \"\"
niche = \"\"
             [schedule]
times = [\"11:00\", \"14:00\"]
jitter_min = 0
"
        ))
        .unwrap();
        ch.paused = paused;
        ch
    }

    fn as_15h() -> DateTime<Local> {
        let d = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap().and_hms_opt(15, 0, 0).unwrap();
        Local.from_local_datetime(&d).earliest().unwrap()
    }

    #[test]
    fn autopilot_late_slot_and_attempts() {
        let store = Store::memory().unwrap();
        let ch = test_channel("piloto_a", false);
        let now = as_15h();
        // 11:00 passou ha 4 h (> 3 h): pulado. 14:00 passou ha 1 h: roda.
        assert_eq!(due_slot(&ch, &store, now), Some(1));
        bump(&store, &ch.slug, now.date_naive(), 1, None);
        assert_eq!(due_slot(&ch, &store, now), Some(1), "1 tentativa: ainda pode");
        bump(&store, &ch.slug, now.date_naive(), 1, None);
        assert_eq!(due_slot(&ch, &store, now), None, "2 tentativas: para");
        // as 13:00 o de 11:00 tem 2 h de atraso: roda; as 10:00 nada venceu
        assert_eq!(due_slot(&test_channel("piloto_b", false), &store, now - CDuration::hours(2)), Some(0));
        assert_eq!(due_slot(&test_channel("piloto_c", false), &store, now - CDuration::hours(5)), None);
    }

    /// E3 do CHAOS.md: reboot no meio do autopilot. O processo morre (e com ele os contadores de tentativa em
    /// memoria); quem decide o horario na volta e so o banco.
    #[test]
    fn autopilot_after_reboot_no_double_no_skip() {
        let store = Store::memory().unwrap();
        let mut ch: Channel = toml::from_str(
            "slug = \"piloto_reboot\"
ref_handle = \"none\"
post_handle = \"\"
persona = \"\"
niche = \"\"
             [schedule]
times = [\"12:30\", \"14:00\"]
jitter_min = 0
",
        )
        .unwrap();
        ch.paused = false;
        let now = as_15h();
        let at = |h: u32, m: u32| {
            let d = now.date_naive().and_hms_opt(h, m, 0).unwrap();
            Local.from_local_datetime(&d).earliest().unwrap().with_timezone(&Utc).to_rfc3339()
        };
        // nada postado: o das 12:30 (2,5 h de atraso) vem primeiro
        assert_eq!(due_slot(&ch, &store, now), Some(0));
        // video A saiu nas duas redes as 12:35: o proximo e o das 14:00 (antes: 2 linhas = 2 posts, pulava o dia)
        for net in ["tiktok", "youtube"] {
            store
                .db()
                .execute(
                    "INSERT INTO posts (channel, video_id, url, caption, posted_at, platform) VALUES (?1, 'A', '', '', ?2, ?3)",
                    rusqlite::params![ch.slug, at(12, 35), net],
                )
                .unwrap();
        }
        assert_eq!(due_slot(&ch, &store, now), Some(1));
        assert_eq!(store.posts_since(&ch.slug, &local_midnight_utc(now)).unwrap(), 1);
        // reboot no meio do upload do video B (reserva ficou, processo morreu): o horario das 14:00 nao roda de novo
        let since = local_midnight_utc(now);
        assert_eq!(store.claim_publish_within(&ch.slug, "tiktok", "B", "cap", None).unwrap(), crate::state::Claim::Fresh);
        store.db().execute("UPDATE publishing SET started_at = ?1, pid = 999999", rusqlite::params![at(14, 5)]).unwrap();
        assert_eq!(due_slot(&ch, &store, now), None, "upload em andamento/sem confirmacao conta como feito");
        store.mark_unconfirmed(&ch.slug, "tiktok", "B", "reboot").unwrap();
        assert_eq!(due_slot(&ch, &store, now), None, "sem confirmacao: a conferencia do Studio decide, nao um video novo");
        assert_eq!(store.posts_since(&ch.slug, &since).unwrap(), 2);
        // a conferencia disse que NAO foi ao ar (reserva liberada): o horario volta a valer
        store.release_publish(&ch.slug, "tiktok", "B").unwrap();
        assert_eq!(due_slot(&ch, &store, now), Some(1));
    }

    /// Reboot com os dois horarios do dia perdidos (12:30 e 14:00, agora 15:00): o primeiro sai e o segundo espera
    /// o intervalo minimo (metade de 90 min = 45 min), em vez de sair minutos depois.
    #[test]
    fn missed_slots_do_not_burst() {
        let store = Store::memory().unwrap();
        let mut ch = test_channel("piloto_rajada", false);
        ch.schedule.times = vec!["12:30".into(), "14:00".into()];
        let now = as_15h();
        assert_eq!(min_post_gap(&ch, now.date_naive()), CDuration::minutes(45));
        assert_eq!(due_slot(&ch, &store, now), Some(0));
        let posted = (now + CDuration::minutes(2)).with_timezone(&Utc).to_rfc3339();
        store
            .db()
            .execute(
                "INSERT INTO posts (channel, video_id, url, caption, posted_at, platform) VALUES (?1, 'A', '', '', ?2, 'tiktok')",
                rusqlite::params![ch.slug, posted],
            )
            .unwrap();
        assert_eq!(due_slot(&ch, &store, now + CDuration::minutes(5)), None, "minutos depois do outro: espera");
        assert_eq!(due_slot(&ch, &store, now + CDuration::minutes(48)), Some(1), "passou o intervalo: sai");
    }

    /// Agenda apertada de proposito (09:30 e 10:00): o intervalo minimo nao derruba o segundo horario.
    #[test]
    fn tight_schedule_still_posts() {
        let mut ch = test_channel("piloto_apertado", false);
        ch.schedule.times = vec!["09:30".into(), "10:00".into()];
        let d = as_15h().date_naive();
        assert_eq!(min_post_gap(&ch, d), CDuration::minutes(15));
        ch.schedule.times = vec!["17:00".into()];
        assert_eq!(min_post_gap(&ch, d), CDuration::minutes(MAX_POST_GAP_MIN), "1 horario so: teto");
    }

    /// Tentativas ficam no banco: outro Store sobre o mesmo arquivo (= app reaberto) ve as mesmas.
    #[test]
    fn attempts_survive_restart() {
        let p = std::env::temp_dir().join(format!("tkd-attempts-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let d = as_15h().date_naive();
        {
            let a = Store::open_at(&p).unwrap();
            bump(&a, "c", d, 1, None);
            bump(&a, "c", d, 1, None);
        }
        let b = Store::open_at(&p).unwrap();
        assert_eq!(attempts(&b, "c", d, 1), 2);
        bump(&b, "c", d, 1, Some(0));
        assert_eq!(attempts(&b, "c", d, 1), 0);
        drop(b);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn autopilot_never_posts_test_or_paused() {
        let store = Store::memory().unwrap();
        let now = as_15h();
        assert_eq!(due_slot(&test_channel("_teste", false), &store, now), None);
        assert_eq!(due_slot(&test_channel("pausado", true), &store, now), None);
        assert_eq!(next_post(&test_channel("_teste", false), &store, now), None);
    }
}
