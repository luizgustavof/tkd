use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

// ---------------------------------------------------------------- agenda

/// Dias da semana na ordem do chrono (seg = 0).
pub const WEEKDAYS: [&str; 7] = ["seg", "ter", "qua", "qui", "sex", "sab", "dom"];

/// `[schedule]` do channel.toml.
///
/// Formato novo: `days` + `times` + `jitter_min`. O antigo (`window_start`/`window_end`
/// + `posts_per_day`) continua aceito e e convertido em horarios na leitura.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    /// Dias em que posta ("seg".."dom"). Vazio = todo dia.
    #[serde(default)]
    pub days: Vec<String>,
    /// Horarios "HH:MM" de cada post do dia.
    #[serde(default)]
    pub times: Vec<String>,
    /// Variacao aleatoria em volta de cada horario (minutos, para mais e para menos).
    #[serde(default = "ten_default")]
    pub jitter_min: u32,
    // ---- formato antigo (so leitura)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub posts_per_day: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub window_start: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub window_end: String,
}

fn ten_default() -> u32 {
    10
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            days: vec![],
            times: vec!["20:00".into()],
            jitter_min: 10,
            posts_per_day: 0,
            window_start: String::new(),
            window_end: String::new(),
        }
    }
}

fn hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

impl Schedule {
    /// Converte janela antiga em horarios: N posts espalhados pela janela, jitter = meia fatia.
    pub fn normalize(&mut self) {
        if self.times.is_empty() {
            if let (Some(a), Some(b)) = (hm(&self.window_start), hm(&self.window_end)) {
                let n = self.posts_per_day.max(1) as i64;
                let mut len = (b - a).num_minutes();
                if len <= 0 {
                    len += 24 * 60;
                }
                let chunk = len / n;
                self.times = (0..n)
                    .map(|k| {
                        let t = a + Duration::minutes(chunk * k + chunk / 2);
                        t.format("%H:%M").to_string()
                    })
                    .collect();
                self.jitter_min = (chunk / 2).clamp(0, 180) as u32;
            } else {
                self.times = vec!["20:00".into()];
            }
        }
        self.times.retain(|t| hm(t).is_some());
        self.times.sort();
        self.times.dedup();
        self.days.retain(|d| WEEKDAYS.contains(&d.as_str()));
        self.posts_per_day = 0;
        self.window_start.clear();
        self.window_end.clear();
    }

    pub fn posts_per_day(&self) -> usize {
        self.times.len().max(1)
    }

    pub fn posts_on(&self, date: chrono::NaiveDate) -> bool {
        self.days.is_empty() || self.days.iter().any(|d| d == WEEKDAYS[date.weekday().num_days_from_monday() as usize])
    }

    /// Horarios nominais (sem jitter) do dia, em hora local.
    pub fn slots_on(&self, date: chrono::NaiveDate) -> Vec<DateTime<Local>> {
        if !self.posts_on(date) {
            return vec![];
        }
        self.times
            .iter()
            .filter_map(|t| hm(t))
            .filter_map(|t| Local.from_local_datetime(&date.and_time(t)).earliest())
            .collect()
    }

    /// Proximo horario nominal a partir de `now` (ate 8 dias a frente).
    pub fn next_slot(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        (0..8).find_map(|d| {
            let date = now.date_naive() + Duration::days(d);
            self.slots_on(date).into_iter().find(|t| *t > now)
        })
    }

    /// Texto curto: "todo dia · 1 por dia", "seg a sex · 2 por dia", "qua, sex e dom".
    pub fn describe(&self) -> String {
        let n_days = if self.days.is_empty() || self.days.len() == 7 {
            "todo dia".to_string()
        } else if self.days == ["seg", "ter", "qua", "qui", "sex"] {
            "seg a sex".to_string()
        } else {
            let mut d: Vec<&str> = WEEKDAYS.iter().copied().filter(|x| self.days.iter().any(|y| y == x)).collect();
            let last = d.pop().unwrap_or_default();
            if d.is_empty() {
                last.to_string()
            } else {
                format!("{} e {last}", d.join(", "))
            }
        };
        format!("{n_days} · {} por dia", self.posts_per_day())
    }
}

// ---------------------------------------------------------------- canal

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Accounts {
    #[serde(default)]
    pub tiktok: String,
    #[serde(default)]
    pub youtube: String,
}

/// Observacao do usuario para o agente do canal. `once` = some depois de usada.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub text: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub once: bool,
}

/// Modelo e esforco de um nivel da precedencia (vazio = herda).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentPick {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentCfg {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, AgentPick>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Channel {
    pub slug: String,
    pub ref_handle: String,
    /// Alias antigo de `accounts.tiktok` (mantido sincronizado).
    #[serde(default)]
    pub post_handle: String,
    #[serde(default)]
    pub accounts: Accounts,
    /// Canais extras para estudar (alem do `ref_handle`).
    #[serde(default)]
    pub inspirations: Vec<String>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub composition: String,
    /// "texto" (padrao: video de texto/cartela pelo ideate) | "edit" (footage + falas reais de um
    /// personagem, pelo recreate)
    #[serde(default)]
    pub format: String,
    /// Personagens do canal de edit (um por video, em rodizio).
    #[serde(default)]
    pub characters: Vec<String>,
    /// Idioma das falas buscadas pro banco do edit: auto | en | ja | pt
    #[serde(default)]
    pub edit_lang: String,
    /// Idioma das legendas do edit: en | pt
    #[serde(default)]
    pub edit_subs: String,
    /// Canal de cortes: links das lives (Twitch, Kick, YouTube) que o clipper assiste.
    #[serde(default)]
    pub streamers: Vec<String>,
    /// Canal de cortes: jogos aceitos (categoria da live). Vazio = qualquer.
    #[serde(default)]
    pub games: Vec<String>,
    #[serde(default)]
    pub persona: String,
    /// Como o canal assina (outro, marca d'agua). Vazio = 1o pedaco do nome em maiusculas.
    #[serde(default)]
    pub brand: String,
    #[serde(default)]
    pub niche: String,
    #[serde(default)]
    pub hashtags: Vec<String>,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub agent: AgentCfg,
    /// Detector de momentos do footage (scripts/moments.py)
    #[serde(default)]
    pub moments: Moments,
    #[serde(skip)]
    pub dir: PathBuf,
}

/// `[moments]` no channel.toml — tudo opcional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Moments {
    /// fight | speech (vazio: speech no modo falas, fight no resto)
    #[serde(default)]
    pub profile: String,
    /// frases de efeito / nomes que valem ponto quando aparecem na fala
    #[serde(default)]
    pub lexicon: Vec<String>,
    /// sobrescreve pesos do perfil, ex.: { audio = 0.4, motion = 0.2 }
    #[serde(default)]
    pub weights: BTreeMap<String, f64>,
}

impl Channel {
    pub fn is_edit(&self) -> bool {
        self.format == "edit"
    }

    pub fn is_clips(&self) -> bool {
        self.format == FORMAT_CLIPS
    }

    pub fn knowledge_dir(&self) -> PathBuf {
        self.dir.join("knowledge")
    }
    pub fn refs_dir(&self) -> PathBuf {
        self.dir.join("refs")
    }
    pub fn ref_meta_dir(&self) -> PathBuf {
        self.knowledge_dir().join("refs")
    }
    pub fn library_dir(&self) -> PathBuf {
        self.dir.join("library")
    }
    pub fn style_card(&self) -> PathBuf {
        self.knowledge_dir().join("style-card.md")
    }
    pub fn preset(&self) -> PathBuf {
        self.knowledge_dir().join("preset.json")
    }
    pub fn avatar(&self, platform: &str) -> PathBuf {
        if platform == "tiktok" {
            self.dir.join("avatar.jpg")
        } else {
            self.dir.join(format!("avatar-{platform}.jpg"))
        }
    }
    /// Canal de teste (prefixo `_`): nunca posta sozinho.
    pub fn is_test(&self) -> bool {
        self.slug.starts_with('_')
    }
    /// `ref_handle` + inspiracoes, sem repetir e sem vazio/"none".
    pub fn ref_handles(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for h in std::iter::once(&self.ref_handle).chain(self.inspirations.iter()) {
            let h = clean_handle(h);
            if !h.is_empty() && h != "none" && !out.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
                out.push(h);
            }
        }
        out
    }
    /// Observacoes em texto para os templates ({{notes}}).
    pub fn notes_text(&self) -> String {
        if self.notes.is_empty() {
            "(nenhuma)".into()
        } else {
            self.notes.iter().map(|n| format!("- {}", n.text)).collect::<Vec<_>>().join("\n")
        }
    }
}

pub fn clean_handle(h: &str) -> String {
    let h = h.trim();
    let h = h.rsplit("tiktok.com/").next().unwrap_or(h);
    h.trim_start_matches('@').trim_end_matches('/').split(['?', '/']).next().unwrap_or("").to_string()
}

// ---------------------------------------------------------------- raiz

static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Aponta o tkd para outra pasta de projeto (usado pelo app desktop).
pub fn set_root(p: impl Into<PathBuf>) {
    if let Ok(mut g) = ROOT.write() {
        *g = Some(p.into());
    }
}

/// Raiz do projeto: `set_root`, $TKD_ROOT ou o diretorio atual.
pub fn root() -> PathBuf {
    if let Some(p) = ROOT.read().ok().and_then(|g| g.clone()) {
        return p;
    }
    std::env::var("TKD_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

// ---------------------------------------------------------------- leitura

fn slug_of(dir: &Path) -> String {
    dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Copia guardada no state.db do channel.toml do canal.
fn saved_config(slug: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    let db = crate::state::side_db().ok()?;
    db.query_row("SELECT toml FROM channel_configs WHERE slug = ?1", [slug], |r| r.get(0)).optional().ok().flatten()
}

/// Guarda a versao valida do channel.toml no state.db (so grava quando mudou).
fn save_config(slug: &str, text: &str) {
    let r = crate::state::side_db().and_then(|db| {
        db.execute(
            "INSERT INTO channel_configs (slug, toml, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(slug) DO UPDATE SET toml = excluded.toml, updated_at = excluded.updated_at
             WHERE channel_configs.toml <> excluded.toml",
            rusqlite::params![slug, text, crate::state::now()],
        )?;
        Ok(())
    });
    if let Err(e) = r {
        tracing::warn!("[{slug}] nao guardei a config no state.db: {e:#}");
    }
}

/// Troca o slug da copia guardada (renomear canal).
fn rename_config(old: &str, new: &str) {
    if let Ok(db) = crate::state::side_db() {
        let _ = db.execute("UPDATE channel_configs SET slug = ?2 WHERE slug = ?1", [old, new]);
    }
}

/// Tem channel.toml no disco ou copia no state.db?
fn has_channel(dir: &Path) -> bool {
    dir.join("channel.toml").exists() || (dir.is_dir() && saved_config(&slug_of(dir)).is_some())
}

/// Le o channel.toml. Arquivo valido = fonte (e vira a copia guardada). Arquivo quebrado ou sumido: usa a copia
/// do state.db; o arquivo quebrado nao e sobrescrito (pode ter edicao a mao para consertar), o sumido e recriado.
fn channel_text(dir: &Path) -> Result<String> {
    let path = dir.join("channel.toml");
    let slug = slug_of(dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<toml::Value>(&text) {
            Ok(_) => {
                save_config(&slug, &text);
                Ok(text)
            }
            Err(e) => match saved_config(&slug) {
                Some(saved) => {
                    tracing::warn!("[{slug}] channel.toml invalido ({e}): usando a ultima versao boa do state.db");
                    Ok(saved)
                }
                None => Err(anyhow::Error::new(e).context(format!("parse de {}", path.display()))),
            },
        },
        Err(e) => match saved_config(&slug) {
            Some(saved) => {
                tracing::warn!("[{slug}] channel.toml sumiu: recriado da copia do state.db");
                let _ = write_atomic(&path, &saved);
                Ok(saved)
            }
            None => Err(anyhow::Error::new(e).context(format!("lendo {}", path.display()))),
        },
    }
}

pub fn load_channel(dir: &Path) -> Result<Channel> {
    let path = dir.join("channel.toml");
    let text = channel_text(dir)?;
    let mut ch: Channel = toml::from_str(&text).with_context(|| format!("parse de {}", path.display()))?;
    ch.dir = dir.to_path_buf();
    if ch.composition.is_empty() {
        ch.composition = DEFAULT_COMPOSITION.into();
    }
    if ch.format == FORMAT_CLIPS_OLD {
        ch.format = FORMAT_CLIPS.into();
    }
    if ch.accounts.tiktok.is_empty() {
        ch.accounts.tiktok = ch.post_handle.clone();
    } else {
        ch.post_handle = ch.accounts.tiktok.clone();
    }
    ch.schedule.normalize();
    Ok(ch)
}

pub fn channels_dir() -> PathBuf {
    root().join("channels")
}

/// Pasta pra onde vao os canais excluidos (fora do git, recuperavel a mao).
pub fn trash_dir() -> PathBuf {
    root().join(".lixeira")
}

/// Exclui um canal: move `channels/<slug>` e o perfil de navegador (logins) pra
/// `.lixeira/<slug>-<data>/`. Historico no banco, analises e clipes ficam.
pub fn delete_channel(slug: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "nome de canal invalido: {slug}"
    );
    let src = channels_dir().join(slug);
    anyhow::ensure!(src.join("channel.toml").exists(), "canal {slug} nao existe");
    let dst = trash_dir().join(format!("{slug}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    std::fs::create_dir_all(&dst)?;
    let profile = crate::accounts::profile_dir(slug);
    if profile.exists() {
        std::fs::rename(&profile, dst.join("profile"))
            .context("o navegador desse canal ainda esta aberto: feche e tente de novo")?;
    }
    if let Err(e) = std::fs::rename(&src, dst.join("channel")) {
        // devolve o perfil pra nao deixar o canal pela metade
        let _ = std::fs::rename(dst.join("profile"), &profile);
        return Err(anyhow::Error::new(e).context(format!("movendo {}", src.display())));
    }
    Ok(dst)
}

/// Troca o nome (slug) de um canal: move a pasta do canal, o perfil do navegador (logins),
/// `video/public/<slug>` e `analysis/<slug>`, e grava o slug novo no channel.toml.
/// Se algum passo falhar, desfaz os que ja moveram. O historico no banco e com o `Store`.
pub fn rename_channel(old: &str, new: &str) -> Result<Channel> {
    anyhow::ensure!(valid_slug(new), "use so letras minusculas, numeros, '-' e '_' no nome");
    anyhow::ensure!(old != new, "o nome novo e igual ao atual");
    let src = channels_dir().join(old);
    anyhow::ensure!(src.join("channel.toml").exists(), "canal {old} nao existe");
    anyhow::ensure!(!channels_dir().join(new).exists(), "Já existe um canal chamado {new}");

    let moves: Vec<(PathBuf, PathBuf, &str)> = vec![
        (crate::accounts::profile_dir(old), crate::accounts::profile_dir(new), "o navegador desse canal ainda esta aberto: feche e tente de novo"),
        (root().join("video").join("public").join(old), root().join("video").join("public").join(new), "pasta de clipes em uso"),
        (root().join("analysis").join(old), root().join("analysis").join(new), "pasta de analises em uso"),
        (src.clone(), channels_dir().join(new), "pasta do canal em uso"),
    ];
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to, why) in &moves {
        if !from.exists() {
            continue;
        }
        if to.exists() {
            for (a, b) in done.iter().rev() {
                let _ = std::fs::rename(b, a);
            }
            anyhow::bail!("{} ja existe", to.display());
        }
        if let Err(e) = std::fs::rename(from, to) {
            for (a, b) in done.iter().rev() {
                let _ = std::fs::rename(b, a);
            }
            return Err(anyhow::Error::new(e).context(*why));
        }
        done.push((from.clone(), to.clone()));
    }
    rename_config(old, new);
    edit_toml(&channels_dir().join(new).join("channel.toml"), |doc| {
        doc["slug"] = toml_edit::value(new);
        Ok(())
    })?;
    get_channel(new)
}

/// Canais de producao (sem os de teste `_`).
pub fn load_channels() -> Result<Vec<Channel>> {
    Ok(load_all_channels()?.into_iter().filter(|c| !c.is_test()).collect())
}

/// Todos, inclusive os de teste (o app mostra todos).
pub fn load_all_channels() -> Result<Vec<Channel>> {
    let base = channels_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&base).with_context(|| format!("lendo {}", base.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && has_channel(&entry.path()) {
            match load_channel(&entry.path()) {
                Ok(c) => out.push(c),
                Err(e) => tracing::error!("{e:#}"),
            }
        }
    }
    out.sort_by(|a, b| a.is_test().cmp(&b.is_test()).then(a.slug.cmp(&b.slug)));
    Ok(out)
}

pub fn get_channel(slug: &str) -> Result<Channel> {
    anyhow::ensure!(valid_slug(slug), "nome de canal invalido: '{slug}'");
    let dir = channels_dir().join(slug);
    anyhow::ensure!(has_channel(&dir), "canal '{slug}' nao existe em channels/");
    load_channel(&dir)
}

/// Canais alvo: um slug especifico ou todos.
pub fn select(slug: Option<&str>) -> Result<Vec<Channel>> {
    match slug {
        Some(s) => Ok(vec![get_channel(s)?]),
        None => load_channels(),
    }
}

pub fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

// ---------------------------------------------------------------- escrita

/// Escreve via arquivo temporario + rename (nunca deixa o arquivo pela metade).
///
/// Falhou no meio (disco cheio, OneDrive/antivirus segurando o arquivo): o temporario e apagado e o
/// original fica como estava. O rename e tentado de novo algumas vezes, porque o OneDrive solta o
/// arquivo depois de alguns centesimos.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    // nome unico: dois escritores com o mesmo .tmp se atropelavam (e o rename falhava no OneDrive)
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{}.{n}.tmp", std::process::id()));
    let cleanup = crate::proc::RemoveOnDrop(tmp.clone());
    write_tmp(&tmp, text).with_context(|| format!("escrevendo {} (o arquivo antigo ficou intacto)", path.display()))?;
    let mut tries = 0;
    loop {
        let r = if crate::chaos::hit(crate::chaos::Kind::Fail, "rename") {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        } else {
            std::fs::rename(&tmp, path)
        };
        match r {
            Ok(()) => break,
            Err(e) if tries < 5 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                tracing::debug!("rename de {} negado (OneDrive?); tentativa {tries}", path.display());
                std::thread::sleep(std::time::Duration::from_millis(150 * tries));
            }
            Err(e) => {
                return Err(e).with_context(|| format!("trocando {} (o arquivo antigo ficou intacto)", path.display()))
            }
        }
    }
    std::mem::forget(cleanup);
    Ok(())
}

fn write_tmp(tmp: &Path, text: &str) -> std::io::Result<()> {
    use crate::chaos::{hit, Kind};
    if hit(Kind::Full, "write") {
        // disco cheio de verdade deixa o comeco escrito: o temporario pela metade tem que sumir
        let _ = std::fs::write(tmp, &text.as_bytes()[..text.len() / 2]);
        return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "nao ha espaco suficiente no disco (caos)"));
    }
    if hit(Kind::Fail, "write") {
        return Err(std::io::Error::other("falha de escrita (caos)"));
    }
    std::fs::write(tmp, text)
}

/// Grava um JSON de conhecimento (clips.json, tracks.json...) de forma atomica, guardando a versao
/// anterior boa em `<arquivo>.bak` para o `load_json` recuperar se o principal corromper.
pub fn save_json<T: Serialize + ?Sized>(path: &Path, v: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(v)?;
    if let Ok(old) = std::fs::read_to_string(path) {
        if old != text && serde_json::from_str::<serde_json::Value>(&old).is_ok() {
            let _ = std::fs::write(bak_path(path), old);
        }
    }
    write_atomic(path, &text)
}

fn bak_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!("{name}.bak"))
}

/// Le um JSON de conhecimento. Ausente = vazio. Corrompido (JSON cortado, lixo) NAO vira "vazio"
/// calado: isso fazia o proximo `save` gravar um banco vazio por cima de tudo. Aqui o arquivo ruim
/// e guardado como `<arquivo>.corrupt-<data>`, a ultima versao boa (`.bak`) volta para o lugar e o
/// problema vai para o log.
pub fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            tracing::error!("nao consegui ler {}: {e}", path.display());
            return T::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => return v,
        Err(e) => tracing::error!("{} corrompido ({e}); procurando a ultima versao boa", path.display()),
    }
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let keep = path.with_file_name(format!("{name}.corrupt-{}", Local::now().format("%Y%m%d-%H%M%S")));
    let kept = match std::fs::copy(path, &keep) {
        Ok(_) => true,
        Err(e) => {
            tracing::error!("nao consegui guardar a copia corrompida em {}: {e}", keep.display());
            false
        }
    };
    let bak = bak_path(path);
    match std::fs::read_to_string(&bak).ok().and_then(|t| serde_json::from_str::<T>(&t).ok().map(|v| (t, v))) {
        Some((t, v)) => {
            match write_atomic(path, &t) {
                Ok(()) => tracing::warn!(
                    "{} restaurado da copia {} (o ruim ficou em {})",
                    path.display(),
                    bak.display(),
                    keep.display()
                ),
                Err(e) => tracing::error!("nao consegui restaurar {}: {e:#}", path.display()),
            }
            v
        }
        None => {
            // sem copia boa: o arquivo ruim sai do lugar para o proximo save nao parecer "banco apagado"
            if kept {
                let _ = std::fs::remove_file(path);
            }
            tracing::error!("{} sem copia boa: comecando vazio (o arquivo ruim ficou em {})", path.display(), keep.display());
            T::default()
        }
    }
}

/// Edita um toml preservando comentarios e ordem.
pub fn edit_toml(path: &Path, f: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>) -> Result<()> {
    // ler-editar-gravar sem trava perdia uma das edicoes quando UI, publish e pausa escreviam juntos
    static EDIT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = EDIT.lock().unwrap_or_else(|e| e.into_inner());
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().with_context(|| format!("parse de {}", path.display()))?;
    f(&mut doc)?;
    write_atomic(path, &doc.to_string())
}

/// Edita o channel.toml de um canal.
pub fn edit_channel(slug: &str, f: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>) -> Result<Channel> {
    let ch = get_channel(slug)?;
    let path = ch.dir.join("channel.toml");
    // arquivo quebrado: edita a partir da ultima versao boa (senao o parse falha e a mudanca da UI se perde)
    if std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str::<toml::Value>(&t).ok()).is_none() {
        if let Some(saved) = saved_config(slug) {
            let _ = std::fs::rename(&path, path.with_extension("toml.invalid"));
            write_atomic(&path, &saved)?;
        }
    }
    edit_toml(&path, f)?;
    get_channel(slug)
}

/// Converte um valor serde em item toml_edit (tabela normal, nao inline).
pub fn to_item<T: Serialize>(v: &T) -> Result<toml_edit::Item> {
    let s = toml::to_string(&Wrap { v })?;
    let doc: toml_edit::DocumentMut = s.parse()?;
    Ok(doc.as_table().get("v").cloned().unwrap_or(toml_edit::Item::None))
}

#[derive(Serialize)]
struct Wrap<'a, T: Serialize> {
    v: &'a T,
}

/// Formato de canal de cortes de live (`format` no channel.toml).
pub const FORMAT_CLIPS: &str = "clips";
/// Nome antigo do formato (antes do rename para ingles): ainda aceito e lido como `FORMAT_CLIPS`.
pub const FORMAT_CLIPS_OLD: &str = "cortes";

/// Composicao generica do Remotion (video/src/Root.tsx) para canal sem composicao propria.
pub const DEFAULT_COMPOSITION: &str = "canal";

/// Grava `key` no documento. Se ja existe uma tabela ali, mescla no lugar: chaves e tabelas
/// que continuam mantem comentarios e posicao; so o que sumiu e removido.
/// (Trocar a tabela inteira apagava os comentarios que o usuario escreveu no toml.)
pub fn set_item(doc: &mut toml_edit::DocumentMut, key: &str, new: toml_edit::Item) {
    match (doc.get_mut(key), new) {
        (Some(toml_edit::Item::Table(old)), toml_edit::Item::Table(new)) => merge_table(old, new),
        (_, new) => doc[key] = new,
    }
}

fn merge_table(old: &mut toml_edit::Table, new: toml_edit::Table) {
    use toml_edit::Item;
    let gone: Vec<String> = old.iter().map(|(k, _)| k.to_string()).filter(|k| !new.contains_key(k)).collect();
    for k in gone {
        old.remove(&k);
    }
    for (k, v) in new {
        match (old.get_mut(&k), v) {
            (Some(Item::Table(o)), Item::Table(n)) => merge_table(o, n),
            (Some(Item::Value(o)), Item::Value(n)) => {
                let decor = o.decor().clone();
                *o = n;
                *o.decor_mut() = decor;
            }
            (_, v) => {
                old.insert(&k, v);
            }
        }
    }
}

/// Cria channels/<slug>/channel.toml.
pub fn create_channel(slug: &str, ref_handle: &str, niche: &str, composition: &str) -> Result<Channel> {
    anyhow::ensure!(valid_slug(slug), "use so letras minusculas, numeros, '-' e '_' no nome");
    let dir = channels_dir().join(slug);
    anyhow::ensure!(!dir.join("channel.toml").exists(), "Já existe um canal chamado {slug}");
    std::fs::create_dir_all(dir.join("knowledge"))?;
    std::fs::create_dir_all(dir.join("library"))?;
    let mut doc = toml_edit::DocumentMut::new();
    doc["slug"] = toml_edit::value(slug);
    doc["ref_handle"] = toml_edit::value(clean_handle(ref_handle));
    doc["post_handle"] = toml_edit::value("");
    doc["composition"] = toml_edit::value(if composition.is_empty() { DEFAULT_COMPOSITION } else { composition });
    doc["niche"] = toml_edit::value(niche);
    doc["persona"] = toml_edit::value("");
    doc["hashtags"] = toml_edit::value(toml_edit::Array::new());
    doc["schedule"] = to_item(&Schedule::default())?;
    write_atomic(&dir.join("channel.toml"), &doc.to_string())?;
    get_channel(slug)
}

// ---------------------------------------------------------------- tkd.toml global

/// `tkd.toml` na raiz: padrao global dos agentes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Global {
    #[serde(default)]
    pub agents: GlobalAgents,
    #[serde(default)]
    pub tasks: BTreeMap<String, AgentPick>,
    #[serde(default)]
    pub app: AppCfg,
}

/// `[app]` do tkd.toml: preferencias do app desktop.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppCfg {
    /// Piloto automatico ligado ao abrir o app.
    #[serde(default)]
    pub autopilot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalAgents {
    #[serde(default = "def_model")]
    pub model: String,
    #[serde(default = "def_effort")]
    pub effort: String,
    /// Chamadas de agente ao mesmo tempo.
    #[serde(default = "two_default")]
    pub parallel: usize,
}

fn def_model() -> String {
    "claude-opus-5-5".into()
}
fn def_effort() -> String {
    "high".into()
}
fn two_default() -> usize {
    2
}

impl Default for GlobalAgents {
    fn default() -> Self {
        Self { model: def_model(), effort: def_effort(), parallel: 2 }
    }
}

impl Default for Global {
    fn default() -> Self {
        Self { agents: GlobalAgents::default(), tasks: BTreeMap::new(), app: AppCfg::default() }
    }
}

pub fn global_path() -> PathBuf {
    root().join("tkd.toml")
}

pub fn load_global() -> Global {
    std::fs::read_to_string(global_path())
        .ok()
        .and_then(|t| match toml::from_str(&t) {
            Ok(g) => Some(g),
            Err(e) => {
                tracing::error!("tkd.toml invalido: {e}");
                None
            }
        })
        .unwrap_or_default()
}

pub fn save_global(g: &Global) -> Result<()> {
    let path = global_path();
    edit_toml(&path, |doc| {
        set_item(doc, "agents", to_item(&g.agents)?);
        set_item(doc, "tasks", to_item(&g.tasks)?);
        set_item(doc, "app", to_item(&g.app)?);
        Ok(())
    })
}

// ---------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_becomes_slot() {
        let mut s = Schedule {
            days: vec![],
            times: vec![],
            jitter_min: 10,
            posts_per_day: 1,
            window_start: "18:00".into(),
            window_end: "22:00".into(),
        };
        s.normalize();
        assert_eq!(s.times, vec!["20:00"]);
        assert_eq!(s.jitter_min, 120);

        let mut s2 = Schedule { posts_per_day: 2, times: vec![], ..s.clone() };
        s2.window_start = "18:00".into();
        s2.window_end = "22:00".into();
        s2.normalize();
        assert_eq!(s2.times, vec!["19:00", "21:00"]);
        assert_eq!(s2.jitter_min, 60);
    }

    #[test]
    fn next_post_at() {
        let s = Schedule { days: vec!["seg".into()], times: vec!["19:30".into()], ..Schedule::default() };
        // 2026-11-16 e segunda
        let now = Local.with_ymd_and_hms(2026, 11, 16, 20, 0, 0).unwrap();
        let n = s.next_slot(now).unwrap();
        assert_eq!(n.date_naive(), chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap());
        let too_early = Local.with_ymd_and_hms(2026, 11, 16, 8, 0, 0).unwrap();
        assert_eq!(s.next_slot(too_early).unwrap().format("%d %H:%M").to_string(), "16 19:30");
    }

    #[test]
    fn describes() {
        let s = Schedule { days: vec!["qua".into(), "sex".into(), "dom".into()], ..Schedule::default() };
        assert_eq!(s.describe(), "qua, sex e dom · 1 por dia");
    }

    #[test]
    fn handles() {
        assert_eq!(clean_handle("https://www.tiktok.com/@seu.canal?lang=pt"), "seu.canal");
        assert_eq!(clean_handle("@jane "), "jane");
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn creates_and_edits_channel() {
        let dir = std::env::temp_dir().join(format!("tkd-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("channels")).unwrap();
        set_root(&dir);

        let ch = create_channel("_novo", "@ref.um", "nicho x", "").unwrap();
        assert_eq!(ch.ref_handle, "ref.um");
        assert_eq!(ch.schedule.times, vec!["20:00"]);

        let notes = vec![
            Note { text: "sempre em minusculo".into(), created_at: "2026-11-16".into(), once: false },
            Note { text: "tema Henry".into(), created_at: String::new(), once: true },
        ];
        let ch = edit_channel("_novo", |doc| {
            set_item(doc, "notes", to_item(&notes)?);
            doc["inspirations"] = toml_edit::value(toml_edit::Array::from_iter(["outro"]));
            let s = Schedule { days: vec!["seg".into(), "sex".into()], times: vec!["22:00".into(), "19:30".into()], jitter_min: 0, ..Default::default() };
            set_item(doc, "schedule", to_item(&s)?);
            let mut a = AgentCfg { model: "claude-sonnet-5-5".into(), ..Default::default() };
            a.tasks.insert("review".into(), AgentPick { model: String::new(), effort: "low".into() });
            set_item(doc, "agent", to_item(&a)?);
            Ok(())
        })
        .unwrap();
        assert_eq!(ch.notes, notes);
        assert_eq!(ch.ref_handles(), vec!["ref.um", "outro"]);
        assert_eq!(ch.schedule.times, vec!["19:30", "22:00"]);
        assert_eq!(ch.schedule.jitter_min, 0);
        assert_eq!(ch.agent.tasks["review"].effort, "low");

        let mut g = load_global();
        g.app.autopilot = true;
        g.tasks.insert("tag_clips".into(), AgentPick { model: "claude-haiku-5-5".into(), effort: String::new() });
        save_global(&g).unwrap();
        assert_eq!(load_global(), g);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod set_item_tests {
    use super::*;

    #[test]
    fn merges_without_losing_comments() {
        let mut doc: toml_edit::DocumentMut = "slug = \"x\"\n# do usuario\n[schedule]\n# hora local\ndays = []\ntimes = [\"11:00\"] # manha\nposts_per_day = 1\n".parse().unwrap();
        let s = Schedule { days: vec!["seg".into()], times: vec!["15:00".into()], jitter_min: 5, ..Default::default() };
        set_item(&mut doc, "schedule", to_item(&s).unwrap());
        let out = doc.to_string();
        assert!(out.contains("# do usuario\n[schedule]"), "{out}");
        assert!(out.contains("# hora local\ndays = [\"seg\"]"), "{out}");
        assert!(out.contains("times = [\"15:00\"] # manha"), "{out}");
        assert!(out.contains("jitter_min = 5"), "{out}");
        assert!(!out.contains("posts_per_day"), "{out}");
        let back: toml::Value = toml::from_str(&out).unwrap();
        let sch: Schedule = back["schedule"].clone().try_into().unwrap();
        assert_eq!(sch.times, ["15:00"]);
    }
}

#[cfg(test)]
mod json_store_tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tkd-json-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn corrupt_json_restores_from_bak() {
        let d = tmpdir("bak");
        let p = d.join("clips.json");
        save_json(&p, &vec![1, 2, 3]).unwrap();
        save_json(&p, &vec![1, 2, 3, 4]).unwrap();
        // JSON cortado no meio (app morreu escrevendo / disco cheio no metodo antigo)
        std::fs::write(&p, "[1, 2, 3, 4").unwrap();
        let v: Vec<i32> = load_json(&p);
        assert_eq!(v, vec![1, 2, 3], "voltou a ultima versao boa");
        assert_eq!(serde_json::from_str::<Vec<i32>>(&std::fs::read_to_string(&p).unwrap()).unwrap(), vec![1, 2, 3]);
        let corrupt = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains(".corrupt-")).count();
        assert_eq!(corrupt, 1, "o arquivo ruim fica guardado");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_json_without_bak_starts_empty_keeps_bad() {
        let d = tmpdir("nobak");
        let p = d.join("tracks.json");
        std::fs::write(&p, "{lixo").unwrap();
        let v: Vec<i32> = load_json(&p);
        assert!(v.is_empty());
        assert!(!p.exists());
        assert!(std::fs::read_dir(&d).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains(".corrupt-")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn atomic_write_leaves_no_tmp() {
        let d = tmpdir("atomic");
        let p = d.join("channel.toml");
        write_atomic(&p, "a = 1").unwrap();
        write_atomic(&p, "a = 2").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "a = 2");
        // destino invalido (pasta no lugar do arquivo): erro e o temporario nao fica largado
        let bad = d.join("pasta");
        std::fs::create_dir_all(bad.join("x")).unwrap();
        assert!(write_atomic(&bad, "b").is_err());
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".tmp")).collect();
        assert!(left.is_empty(), "{left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
