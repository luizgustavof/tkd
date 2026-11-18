//! Tipos que a tela recebe (o TS e gerado deles pelo specta).

use chrono::{DateTime, Datelike, Local, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use tkd_core::config::Channel;
use tkd_core::events::Event;
use tkd_core::state::Store;

/// Etapas do dia do agente (a barrinha da tabela de canais).
pub const STAGES: [(&str, &str); 6] = [
    ("scout", "Buscar referências"),
    ("study", "Estudar"),
    ("ideate", "Criar a ideia"),
    ("render", "Montar o vídeo"),
    ("review", "Avaliar"),
    ("publish", "Postar"),
];

pub fn step_label(step: &str) -> String {
    if let Some(who) = step.strip_prefix("live:") {
        return format!("Live de {who}");
    }
    match step {
        "scout" => "Buscando referências",
        "study" => "Estudando as referências",
        "ideate" => "Criando a ideia",
        "render" => "Montando o vídeo",
        "review" => "Avaliando o vídeo",
        "publish" => "Postando",
        "footage" => "Buscando imagens",
        "recreate" => "Recriando um edit",
        "login" => "Entrando na conta",
        "live" => "Assistindo live",
        "browser" => "Usando o navegador",
        "agent" => "Pensando",
        _ => "Trabalhando",
    }
    .to_string()
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Stage {
    pub key: String,
    pub label: String,
    /// todo | done | now | posted | error
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct NowState {
    pub label: String,
    /// run | ok | warn | idle | error | off
    pub tone: String,
    pub progress: Option<f64>,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChannelSummary {
    pub slug: String,
    pub niche: String,
    pub is_test: bool,
    pub paused: bool,
    pub busy: bool,
    pub tiktok: String,
    pub youtube: String,
    pub has_profile: bool,
    pub has_avatar: bool,
    /// cor de fundo do avatar com inicial (estavel por canal)
    pub color: String,
    /// canal usa modelo diferente do padrao global
    pub own_model: bool,
    pub now: NowState,
    /// rfc3339 do proximo post, se houver
    pub next_post: Option<String>,
    /// "Hoje, 19:30" | "Amanhã, 19:00" | "Não posta"
    pub next_post_label: String,
    /// "seg a sex · 2 por dia"
    pub schedule_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ScheduleDto {
    pub days: Vec<String>,
    pub times: Vec<String>,
    pub jitter_min: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct NoteDto {
    pub index: u32,
    pub text: String,
    pub created_at: String,
    pub once: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TaskPick {
    pub task: String,
    pub label: String,
    pub model: String,
    pub model_label: String,
    pub effort: String,
    /// global | global_task | channel | channel_task
    pub model_from: String,
    pub effort_from: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HandleRefs {
    pub handle: String,
    /// videos baixados e analisados (entram no estudo)
    pub studied: i32,
    /// videos vistos na grade do perfil
    pub seen: i32,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct Learned {
    pub has_style_card: bool,
    /// primeiras linhas do style card
    pub style_excerpt: String,
    pub refs_total: i32,
    pub refs_analyzed: i32,
    /// contagem por perfil estudado
    pub refs_by_handle: Vec<HandleRefs>,
    pub library: i32,
    pub avg_score: Option<f64>,
    pub last_problems: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RunDto {
    pub run_id: String,
    pub status: String,
    pub detail: String,
    pub at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChannelDetail {
    pub summary: ChannelSummary,
    pub persona: String,
    /// como o canal assina (ja resolvida: brand do toml ou o padrao pelo nome)
    pub brand: String,
    pub hashtags: Vec<String>,
    /// "texto" | "edit"
    pub format: String,
    pub characters: Vec<String>,
    /// idioma das falas do edit: auto | en | ja | pt
    pub edit_lang: String,
    /// canal de cortes: links das lives
    pub streamers: Vec<String>,
    /// canal de cortes: jogos aceitos (categoria da live)
    pub games: Vec<String>,
    /// lives sendo assistidas agora (todas as contas)
    pub watching: Vec<String>,
    pub ref_handle: String,
    pub inspirations: Vec<String>,
    pub notes: Vec<NoteDto>,
    pub schedule: ScheduleDto,
    pub agent_model: String,
    pub agent_effort: String,
    pub picks: Vec<TaskPick>,
    pub learned: Learned,
    pub runs: Vec<RunDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct NewChannel {
    pub slug: String,
    pub ref_handle: String,
    pub niche: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct ChannelPatch {
    pub niche: Option<String>,
    pub persona: Option<String>,
    pub brand: Option<String>,
    pub hashtags: Option<Vec<String>>,
    pub format: Option<String>,
    pub characters: Option<Vec<String>>,
    pub edit_lang: Option<String>,
    pub streamers: Option<Vec<String>>,
    pub games: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SessionCheck {
    pub logged_in: bool,
    pub handle: String,
}

// ---------------------------------------------------------------- agentes

#[derive(Debug, Clone, Serialize, Type)]
pub struct Opt {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TaskRow {
    pub task: String,
    pub label: String,
    /// vazio = herda
    pub model: String,
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ChannelAgent {
    pub slug: String,
    pub color: String,
    pub model: String,
    pub effort: String,
    pub tasks: Vec<TaskRow>,
    pub picks: Vec<TaskPick>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AgentConfig {
    pub model: String,
    pub effort: String,
    pub parallel: u32,
    pub tasks: Vec<TaskRow>,
    pub channels: Vec<ChannelAgent>,
    pub models: Vec<Opt>,
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AgentScope {
    /// None = padrao global
    pub channel: Option<String>,
    /// None = todas as tarefas daquele nivel
    pub task: Option<String>,
}

// ---------------------------------------------------------------- consumo

#[derive(Debug, Clone, Serialize, Type)]
pub struct UsageRowDto {
    pub channel: String,
    pub task: String,
    pub task_label: String,
    pub calls: f64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// input + output + cache_write (o que nao e releitura)
    pub fresh: f64,
    pub total: f64,
    pub cost: f64,
    pub turns: f64,
    pub ms: f64,
}

impl From<&tkd_core::state::UsageRow> for UsageRowDto {
    fn from(r: &tkd_core::state::UsageRow) -> Self {
        let fresh = (r.input + r.output + r.cache_write) as f64;
        Self {
            channel: r.channel.clone(),
            task: r.task.clone(),
            task_label: tkd_core::agent::task_label(&r.task).to_string(),
            calls: r.calls as f64,
            input: r.input as f64,
            output: r.output as f64,
            cache_read: r.cache_read as f64,
            cache_write: r.cache_write as f64,
            fresh,
            total: fresh + r.cache_read as f64,
            cost: r.cost,
            turns: r.turns as f64,
            ms: r.ms as f64,
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DayUsage {
    pub day: String,
    pub fresh: f64,
    pub cache_read: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct UsageSummary {
    pub days: i32,
    pub total: UsageRowDto,
    pub by_channel: Vec<UsageRowDto>,
    pub by_task: Vec<UsageRowDto>,
    pub by_pair: Vec<UsageRowDto>,
    pub by_model: Vec<UsageRowDto>,
    pub daily: Vec<DayUsage>,
    pub insights: Vec<String>,
    /// tokens medios por chamada antes x depois do conserto de contexto (16/11/2026)
    pub before_after: Vec<TaskCompareDto>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct TaskCompareDto {
    pub task: String,
    pub task_label: String,
    pub before_calls: f64,
    pub before_avg: f64,
    pub after_calls: f64,
    pub after_avg: f64,
}

// ---------------------------------------------------------------- ao vivo

#[derive(Debug, Clone, Serialize, Type)]
pub struct EventDto {
    pub run_id: String,
    pub channel: String,
    pub step: String,
    pub step_label: String,
    pub action: String,
    pub tool: String,
    pub status: String,
    pub progress: Option<f64>,
    pub detail: String,
    pub at: String,
}

impl From<&Event> for EventDto {
    fn from(e: &Event) -> Self {
        Self {
            run_id: e.run_id.clone(),
            channel: e.channel.clone(),
            step: e.step.clone(),
            step_label: step_label(&e.step),
            action: e.action.clone(),
            tool: e.tool.clone(),
            status: e.status.clone(),
            progress: e.progress,
            detail: e.detail.clone(),
            at: e.at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct LiveBatch {
    /// true no primeiro envio (estado inicial completo)
    pub initial: bool,
    pub events: Vec<EventDto>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BrowserHolder {
    pub channel: String,
    pub script: String,
    pub since: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BrowserStatus {
    pub holder: Option<BrowserHolder>,
    pub waiting: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct AppStatus {
    pub root: String,
    pub autopilot: bool,
    pub busy: Vec<String>,
    pub model_label: String,
    pub effort: String,
}

/// Um item da conferencia de primeira execucao.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SetupItem {
    pub id: String,
    pub label: String,
    /// "ok" | "warn" | "missing"
    pub status: String,
    pub detail: String,
    pub why: String,
    pub command: Option<String>,
    pub fix: Option<String>,
}

impl From<tkd_core::setup::Item> for SetupItem {
    fn from(i: tkd_core::setup::Item) -> Self {
        let status = match i.status {
            tkd_core::setup::Status::Ok => "ok",
            tkd_core::setup::Status::Warn => "warn",
            tkd_core::setup::Status::Missing => "missing",
        };
        Self {
            id: i.id.into(),
            label: i.label.into(),
            status: status.into(),
            detail: i.detail,
            why: i.why.into(),
            command: i.command,
            fix: i.fix.map(String::from),
        }
    }
}

// ---------------------------------------------------------------- montagem

/// Cor de fundo estavel do avatar com inicial.
pub fn color_for(slug: &str) -> String {
    const P: [&str; 8] = ["#4a2f45", "#2a3550", "#3d2526", "#2b3a33", "#3a3524", "#2f2b4a", "#24383d", "#3b2c22"];
    if slug.starts_with('_') {
        return "#2b2d31".into();
    }
    let h = slug.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    P[(h as usize) % P.len()].into()
}

fn hm_local(rfc: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(rfc).ok().map(|t| t.with_timezone(&Local))
}

pub fn when_label(t: DateTime<Local>, now: DateTime<Local>) -> String {
    let d = (t.date_naive() - now.date_naive()).num_days();
    let hm = t.format("%H:%M");
    match d {
        0 => format!("Hoje, {hm}"),
        1 => format!("Amanhã, {hm}"),
        2..=6 => {
            let weekday_name = tkd_core::config::WEEKDAYS[t.weekday().num_days_from_monday() as usize];
            format!("{weekday_name}, {hm}")
        }
        _ => t.format("%d/%m, %H:%M").to_string(),
    }
}

/// Estado "Agora" de um canal a partir dos eventos ao vivo e do banco.
pub fn now_state(ch: &Channel, store: &Store, busy: bool, events: &[Event]) -> NowState {
    let mine: Vec<&Event> = events.iter().filter(|e| e.channel == ch.slug).collect();
    let mut stages: Vec<Stage> = STAGES
        .iter()
        .map(|(k, l)| Stage { key: k.to_string(), label: l.to_string(), state: "todo".into() })
        .collect();
    let set = |stages: &mut Vec<Stage>, key: &str, st: &str| {
        if let Some(s) = stages.iter_mut().find(|s| s.key == key) {
            s.state = st.into();
        }
    };
    if ch.style_card().exists() {
        set(&mut stages, "scout", "done");
        set(&mut stages, "study", "done");
    }

    let now = Local::now();
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).and_then(|d| d.and_local_timezone(Local).earliest());
    let posted_today = midnight
        .and_then(|m| store.posts_since(&ch.slug, &m.with_timezone(&Utc).to_rfc3339()).ok())
        .unwrap_or(0);

    if busy || mine.iter().any(|e| matches!(e.status.as_str(), "start" | "progress") && STAGES.iter().any(|(k, _)| *k == e.step)) {
        let run = mine.iter().rev().find(|e| !e.run_id.is_empty()).map(|e| e.run_id.clone()).unwrap_or_default();
        let mut active: Option<&Event> = None;
        for e in mine.iter().filter(|e| run.is_empty() || e.run_id == run) {
            if !STAGES.iter().any(|(k, _)| *k == e.step) {
                continue;
            }
            match e.status.as_str() {
                "ok" => set(&mut stages, &e.step, "done"),
                "error" => set(&mut stages, &e.step, "error"),
                "start" | "progress" => {
                    set(&mut stages, &e.step, "now");
                    active = Some(e);
                }
                _ => {}
            }
        }
        // o progresso pode vir do navegador / agente dentro da etapa
        let progress = mine
            .iter()
            .rev()
            .find(|e| matches!(e.status.as_str(), "progress") && e.progress.is_some())
            .and_then(|e| e.progress)
            .filter(|_| active.is_some());
        let label = active.map(|e| step_label(&e.step)).unwrap_or_else(|| "Trabalhando".into());
        let label = match progress {
            Some(p) if active.is_some_and(|a| a.step == "render") => format!("{label} {p:.0}%"),
            _ => label,
        };
        return NowState { label, tone: "run".into(), progress, stages };
    }

    if posted_today > 0 {
        for s in stages.iter_mut() {
            s.state = "done".into();
        }
        set(&mut stages, "publish", "posted");
        let at = store
            .last_post_at(&ch.slug)
            .ok()
            .flatten()
            .and_then(|t| hm_local(&t))
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default();
        return NowState { label: format!("Postado às {at}"), tone: "ok".into(), progress: None, stages };
    }
    if ch.paused {
        return NowState { label: "Pausado".into(), tone: "off".into(), progress: None, stages };
    }
    if ch.is_test() {
        return NowState { label: "Parado".into(), tone: "off".into(), progress: None, stages };
    }
    if !ch.style_card().exists() {
        return NowState { label: "Ainda não estudou".into(), tone: "warn".into(), progress: None, stages };
    }
    if let Ok(Some((step, status, detail, _))) = store.last_run(&ch.slug) {
        if step == "run" && status == "erro" {
            return NowState { label: "Último vídeo deu erro".into(), tone: "error".into(), progress: None, stages };
        }
        if status == "reprovado" {
            let rating = detail.split("nota ").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("").to_string();
            let label = if rating.is_empty() { "Último vídeo reprovado".to_string() } else { format!("Último vídeo reprovado ({rating})") };
            return NowState { label, tone: "warn".into(), progress: None, stages };
        }
    }
    NowState { label: "Esperando o horário".into(), tone: "idle".into(), progress: None, stages }
}

pub fn summary(ch: &Channel, store: &Store, busy: &[String], events: &[Event], global_model: &str) -> ChannelSummary {
    let is_busy = busy.contains(&ch.slug);
    let now = Local::now();
    let next = tkd_core::watch::next_post(ch, store, now);
    let next_post_label = match next {
        Some(t) => when_label(t, now),
        None if ch.is_test() => "Não posta".into(),
        None if ch.paused => "Pausado".into(),
        None => "Sem horário".into(),
    };
    let schedule_label = if ch.is_test() { "canal de teste".to_string() } else { ch.schedule.describe() };
    ChannelSummary {
        slug: ch.slug.clone(),
        niche: ch.niche.clone(),
        is_test: ch.is_test(),
        paused: ch.paused,
        busy: is_busy,
        tiktok: ch.accounts.tiktok.clone(),
        youtube: ch.accounts.youtube.clone(),
        has_profile: tkd_core::accounts::has_profile(&ch.slug),
        has_avatar: ch.avatar("tiktok").exists() || ch.avatar("youtube").exists(),
        color: color_for(&ch.slug),
        own_model: (!ch.agent.model.is_empty() && ch.agent.model != global_model) || !ch.agent.tasks.is_empty(),
        now: now_state(ch, store, is_busy, events),
        next_post: next.map(|t| t.to_rfc3339()),
        next_post_label,
        schedule_label,
    }
}

pub fn picks(ch: Option<&Channel>, global: &tkd_core::config::Global) -> Vec<TaskPick> {
    tkd_core::agent::TASKS
        .iter()
        .map(|(task, label)| {
            let r = tkd_core::agent::resolve_with(global, ch.map(|c| &c.agent), task);
            TaskPick {
                task: task.to_string(),
                label: label.to_string(),
                model_label: tkd_core::agent::model_label(&r.model),
                model: r.model,
                effort: r.effort,
                model_from: r.model_from,
                effort_from: r.effort_from,
            }
        })
        .collect()
}

pub fn task_rows(map: &std::collections::BTreeMap<String, tkd_core::config::AgentPick>) -> Vec<TaskRow> {
    tkd_core::agent::TASKS
        .iter()
        .map(|(task, label)| {
            let p = map.get(*task).cloned().unwrap_or_default();
            TaskRow { task: task.to_string(), label: label.to_string(), model: p.model, effort: p.effort }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ClipDto {
    pub key: String,
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
    pub used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ClipsView {
    /// lives sendo vigiadas agora (urls normalizadas)
    pub watching: Vec<String>,
    pub max_watch: u32,
    pub clips: Vec<ClipDto>,
}
