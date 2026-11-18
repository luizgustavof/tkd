use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct RefVideo {
    pub id: String,
    pub channel: String,
    pub url: String,
    pub caption: String,
    pub stats: String,
    pub downloaded: bool,
    pub analyzed: bool,
}

/// Quanto um comando espera o state.db travado por outro processo.
pub const BUSY_SECS: u64 = 30;

pub fn db_path() -> PathBuf {
    crate::config::root().join("state.db")
}

/// Tabelas de config/estado que antes eram arquivo (channel.toml, knowledge/cortes.json).
const SIDE_TABLES: &str = r#"
    -- ultima versao valida do channel.toml de cada canal: o arquivo segue editavel a mao; arquivo quebrado
    -- (parse falhou) ou apagado carrega daqui em vez de derrubar o canal
    CREATE TABLE IF NOT EXISTS channel_configs (
        slug       TEXT PRIMARY KEY,
        toml       TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    -- cortes de live ja usados por um canal de cortes (antes knowledge/cortes.json, relido inteiro a cada rodada)
    CREATE TABLE IF NOT EXISTS clip_uses (
        channel TEXT NOT NULL,
        clip    TEXT NOT NULL,
        id      TEXT NOT NULL DEFAULT '',
        result  TEXT NOT NULL,
        at      TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS clip_uses_channel ON clip_uses(channel, at);
"#;

/// Conexao curta ao state.db fora do `Store` (config e cortes sao sincronos e chamados de todo lugar).
/// Garante as tabelas. Varios processos (app + CLI) usam o mesmo arquivo: SQLite serializa as escritas.
pub fn side_db() -> Result<Connection> {
    let path = db_path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let conn = Connection::open(&path).with_context(|| format!("abrindo {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(BUSY_SECS))?;
    conn.execute_batch(SIDE_TABLES)?;
    Ok(conn)
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::open_at(&db_path())
    }

    /// Abre (ou cria) o banco num caminho dado.
    pub fn open_at(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("abrindo {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // OneDrive/antivirus seguram o arquivo por mais de 10 s as vezes: espera ate 30 s antes de desistir
        conn.busy_timeout(std::time::Duration::from_secs(BUSY_SECS))?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate().map_err(|e| {
            if format!("{e:#}").contains("locked") {
                anyhow::anyhow!(
                    "state.db ocupado por outro programa ha {BUSY_SECS} s (OneDrive sincronizando? outro tkd gravando?): tente de novo"
                )
            } else {
                e
            }
        })?;
        Ok(store)
    }

    /// Banco em memoria (testes).
    pub fn memory() -> Result<Self> {
        let store = Self { conn: Mutex::new(Connection::open_in_memory()?) };
        store.migrate()?;
        Ok(store)
    }

    /// Conexao (trava curta; nunca segurar atraves de `.await`).
    pub fn db(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn migrate(&self) -> Result<()> {
        self.db().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS ref_videos (
                id          TEXT PRIMARY KEY,
                channel     TEXT NOT NULL,
                url         TEXT NOT NULL,
                caption     TEXT NOT NULL DEFAULT '',
                stats       TEXT NOT NULL DEFAULT '{}',
                seen_at     TEXT NOT NULL,
                downloaded  INTEGER NOT NULL DEFAULT 0,
                analyzed    INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS ref_videos_channel ON ref_videos(channel);

            CREATE TABLE IF NOT EXISTS runs (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                channel   TEXT NOT NULL,
                step      TEXT NOT NULL,
                status    TEXT NOT NULL,
                detail    TEXT NOT NULL DEFAULT '',
                at        TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS videos (
                id        TEXT PRIMARY KEY,
                channel   TEXT NOT NULL,
                spec      TEXT NOT NULL,
                mp4       TEXT NOT NULL DEFAULT '',
                score     REAL,
                kept      INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS posts (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                channel    TEXT NOT NULL,
                video_id   TEXT NOT NULL,
                url        TEXT NOT NULL DEFAULT '',
                caption    TEXT NOT NULL DEFAULT '',
                posted_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS posts_channel ON posts(channel, posted_at);
            CREATE INDEX IF NOT EXISTS runs_channel ON runs(channel, at);

            CREATE TABLE IF NOT EXISTS agent_calls (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                channel     TEXT NOT NULL,
                task        TEXT NOT NULL,
                model       TEXT NOT NULL DEFAULT '',
                effort      TEXT NOT NULL DEFAULT '',
                input       INTEGER NOT NULL DEFAULT 0,
                output      INTEGER NOT NULL DEFAULT 0,
                cache_read  INTEGER NOT NULL DEFAULT 0,
                cache_write INTEGER NOT NULL DEFAULT 0,
                cost        REAL NOT NULL DEFAULT 0,
                turns       INTEGER NOT NULL DEFAULT 0,
                ms          INTEGER NOT NULL DEFAULT 0,
                ok          INTEGER NOT NULL DEFAULT 1,
                source      TEXT NOT NULL DEFAULT 'tkd',
                session     TEXT NOT NULL DEFAULT '',
                at          TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS agent_calls_channel ON agent_calls(channel, at);
            CREATE INDEX IF NOT EXISTS agent_calls_at ON agent_calls(at);
            CREATE UNIQUE INDEX IF NOT EXISTS agent_calls_session ON agent_calls(session) WHERE session != '';

            CREATE TABLE IF NOT EXISTS events (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id    TEXT NOT NULL DEFAULT '',
                channel   TEXT NOT NULL,
                step      TEXT NOT NULL,
                action    TEXT NOT NULL DEFAULT '',
                tool      TEXT NOT NULL DEFAULT '',
                status    TEXT NOT NULL,
                progress  REAL,
                detail    TEXT NOT NULL DEFAULT '',
                at        TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS events_channel ON events(channel, at);

            -- cada avaliacao: nota do modelo local (v4), do Claude (se chamado) e quem decidiu
            CREATE TABLE IF NOT EXISTS reviews (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                channel      TEXT NOT NULL,
                video_id     TEXT NOT NULL,
                local_score  REAL,
                claude_score REAL,
                final_score  REAL NOT NULL,
                kept         INTEGER NOT NULL,
                decided_by   TEXT NOT NULL,
                reason       TEXT NOT NULL DEFAULT '',
                at           TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS reviews_channel ON reviews(channel, at);

            -- publicacao em andamento ou sem confirmacao: gravada ANTES do upload. Se o app morrer entre o
            -- upload e o record_post, a linha fica aqui e a proxima tentativa confere o perfil antes de
            -- reenviar (senao o video saia duas vezes). Tambem e a trava entre app e CLI.
            -- sessao da rede caiu numa postagem (SESSAO_EXPIRADA): o autopilot nao faz video so para
            -- descobrir isso de novo. Volta a valer com login novo ou post que deu certo.
            CREATE TABLE IF NOT EXISTS sessions (
                channel  TEXT NOT NULL,
                platform TEXT NOT NULL,
                expired  INTEGER NOT NULL,
                at       TEXT NOT NULL,
                PRIMARY KEY (channel, platform)
            );

            CREATE TABLE IF NOT EXISTS publishing (
                channel    TEXT NOT NULL,
                platform   TEXT NOT NULL,
                video_id   TEXT NOT NULL,
                state      TEXT NOT NULL,
                pid        INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                caption    TEXT NOT NULL DEFAULT '',
                detail     TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (channel, platform, video_id)
            );

            -- tentativas por horario do dia (autopilot): reprovacao nao vira loop de tokens. Antes ficavam
            -- so em memoria e zeravam no reboot (mais tentativas que o limite no mesmo horario).
            CREATE TABLE IF NOT EXISTS slot_attempts (
                channel TEXT NOT NULL,
                day     TEXT NOT NULL,
                idx     INTEGER NOT NULL,
                n       INTEGER NOT NULL,
                PRIMARY KEY (channel, day, idx)
            );
            "#,
        )?;
        self.db().execute_batch(SIDE_TABLES)?;
        // colunas novas em tabelas antigas (ALTER falha se ja existe: ignorar)
        for sql in [
            "ALTER TABLE runs ADD COLUMN run_id TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE runs ADD COLUMN ended_at TEXT",
            "ALTER TABLE posts ADD COLUMN platform TEXT NOT NULL DEFAULT 'tiktok'",
            // post de teste (Somente voce / nao listado): nao conta no limite do dia, horarios nem intervalo
            "ALTER TABLE posts ADD COLUMN test INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE publishing ADD COLUMN test INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = self.db().execute(sql, []);
        }
        // close_open_runs filtra por run_id: sem indice era varredura da tabela inteira por run aberto
        self.db().execute_batch(
            "CREATE INDEX IF NOT EXISTS events_run ON events(run_id, step, id);
             CREATE INDEX IF NOT EXISTS runs_run ON runs(run_id, step);",
        )?;
        self.prune_events()
    }

    /// Eventos ao vivo so valem por uma semana.
    pub fn prune_events(&self) -> Result<()> {
        self.db().execute("DELETE FROM events WHERE at < ?1", params![days_ago(7)])?;
        Ok(())
    }


    pub fn log(&self, channel: &str, step: &str, status: &str, detail: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO runs (channel, step, status, detail, at) VALUES (?1,?2,?3,?4,?5)",
            params![channel, step, status, detail, now()],
        )?;
        Ok(())
    }

    pub fn ref_exists(&self, id: &str) -> Result<bool> {
        let n: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM ref_videos WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Ja foi baixado com sucesso? (nao-baixados voltam para a fila do scout)
    pub fn ref_downloaded(&self, id: &str) -> Result<bool> {
        let v: Option<i32> = self.db().query_row(
                "SELECT downloaded FROM ref_videos WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0) != 0)
    }

    pub fn insert_ref(&self, v: &RefVideo) -> Result<()> {
        self.db().execute(
            "INSERT OR IGNORE INTO ref_videos (id, channel, url, caption, stats, seen_at, downloaded, analyzed)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![v.id, v.channel, v.url, v.caption, v.stats, now(), v.downloaded as i32, v.analyzed as i32],
        )?;
        Ok(())
    }

    pub fn mark_ref(&self, id: &str, downloaded: bool, analyzed: bool) -> Result<()> {
        self.db().execute(
            "UPDATE ref_videos SET downloaded = ?2, analyzed = ?3 WHERE id = ?1",
            params![id, downloaded as i32, analyzed as i32],
        )?;
        Ok(())
    }

    pub fn pending_refs(&self, channel: &str) -> Result<Vec<RefVideo>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT id, channel, url, caption, stats, downloaded, analyzed
             FROM ref_videos WHERE channel = ?1 AND analyzed = 0",
        )?;
        let rows = stmt
            .query_map(params![channel], |r| {
                Ok(RefVideo {
                    id: r.get(0)?,
                    channel: r.get(1)?,
                    url: r.get(2)?,
                    caption: r.get(3)?,
                    stats: r.get(4)?,
                    downloaded: r.get::<_, i32>(5)? != 0,
                    analyzed: r.get::<_, i32>(6)? != 0,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Por perfil (o @ vem da url do video): (handle, estudados, vistos na grade).
    pub fn refs_by_handle(&self, channel: &str) -> Result<Vec<(String, i64, i64)>> {
        let db = self.db();
        let mut st = db.prepare("SELECT url, analyzed FROM ref_videos WHERE channel = ?1")?;
        let rows = st.query_map(params![channel], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out: Vec<(String, i64, i64)> = Vec::new();
        for row in rows {
            let (url, analyzed) = row?;
            let Some(h) = url.split("/@").nth(1).and_then(|r| r.split('/').next()) else { continue };
            let h = h.to_lowercase();
            match out.iter_mut().find(|x| x.0 == h) {
                Some(x) => {
                    x.1 += analyzed;
                    x.2 += 1;
                }
                None => out.push((h, analyzed, 1)),
            }
        }
        Ok(out)
    }

    pub fn count_refs(&self, channel: &str) -> Result<(i64, i64)> {
        let total: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM ref_videos WHERE channel = ?1",
            params![channel],
            |r| r.get(0),
        )?;
        let analyzed: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM ref_videos WHERE channel = ?1 AND analyzed = 1",
            params![channel],
            |r| r.get(0),
        )?;
        Ok((total, analyzed))
    }

    pub fn record_video(&self, id: &str, channel: &str, spec: &str, mp4: &str) -> Result<()> {
        self.db().execute(
            "INSERT OR REPLACE INTO videos (id, channel, spec, mp4, created_at) VALUES (?1,?2,?3,?4,?5)",
            params![id, channel, spec, mp4, now()],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_review(
        &self,
        channel: &str,
        video_id: &str,
        local: Option<f64>,
        claude: Option<f64>,
        fin: f64,
        kept: bool,
        decided_by: &str,
        reason: &str,
    ) -> Result<()> {
        self.db().execute(
            "INSERT INTO reviews (channel, video_id, local_score, claude_score, final_score, kept, decided_by, reason, at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![channel, video_id, local, claude, fin, kept as i32, decided_by, reason, now()],
        )?;
        Ok(())
    }

    pub fn score_video(&self, id: &str, score: f64, kept: bool) -> Result<()> {
        self.db().execute(
            "UPDATE videos SET score = ?2, kept = ?3 WHERE id = ?1",
            params![id, score, kept as i32],
        )?;
        Ok(())
    }

    pub fn record_post(&self, channel: &str, platform: &str, video_id: &str, url: &str, caption: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO posts (channel, platform, video_id, url, caption, posted_at, test) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![channel, platform, video_id, url, caption, now(), crate::publish::test_post_for(channel) as i64],
        )?;
        Ok(())
    }

    /// Quantos videos o canal postou nas ultimas 24h (o mesmo video no TikTok e no YouTube conta 1).
    ///
    /// Publicacao sem confirmacao (app morreu no meio) conta como postada: pode estar no ar, e o
    /// autopilot nao pode dobrar os posts do dia por causa dela.
    pub fn posts_last_24h(&self, channel: &str) -> Result<i64> {
        // posted_at e RFC 3339 ("...T...+00:00"): compara como data, nao como texto
        let n: i64 = self.db().query_row(
            "SELECT COUNT(DISTINCT video_id) FROM (
                SELECT video_id FROM posts WHERE channel = ?1 AND test = 0 AND datetime(posted_at) > datetime('now','-1 day')
                UNION ALL
                SELECT video_id FROM publishing WHERE channel = ?1 AND test = 0 AND datetime(started_at) > datetime('now','-1 day')
             )",
            params![channel],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Grava se a sessao da rede vale (`ok`) ou caiu.
    pub fn set_session(&self, channel: &str, platform: &str, ok: bool) -> Result<()> {
        self.db().execute(
            "INSERT INTO sessions (channel, platform, expired, at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(channel, platform) DO UPDATE SET expired = excluded.expired, at = excluded.at",
            params![channel, platform, !ok as i32, now()],
        )?;
        Ok(())
    }

    /// A sessao dessa rede caiu numa postagem e ainda nao houve login novo?
    pub fn session_expired(&self, channel: &str, platform: &str) -> Result<bool> {
        let v: Option<i64> = self
            .db()
            .query_row(
                "SELECT expired FROM sessions WHERE channel = ?1 AND platform = ?2",
                params![channel, platform],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v == Some(1))
    }

    /// Reserva a publicacao de um video numa rede ANTES de subir (transacao IMMEDIATE: vale entre processos).
    pub fn claim_publish(&self, channel: &str, platform: &str, video_id: &str, caption: &str) -> Result<Claim> {
        self.claim_publish_within(channel, platform, video_id, caption, None)
    }

    /// `claim_publish` que tambem confere o limite diario NA MESMA transacao: dois processos (app + CLI)
    /// postando videos diferentes do mesmo canal passavam os dois pelo `can_post` antes de qualquer um gravar.
    /// Completar outra rede de um video ja postado/reservado nao conta como video novo.
    pub fn claim_publish_within(
        &self,
        channel: &str,
        platform: &str,
        video_id: &str,
        caption: &str,
        daily_limit: Option<i64>,
    ) -> Result<Claim> {
        let me = std::process::id();
        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let posted: i64 = tx.query_row(
            "SELECT COUNT(*) FROM posts WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
            params![channel, platform, video_id],
            |r| r.get(0),
        )?;
        if posted > 0 {
            return Ok(Claim::AlreadyPosted);
        }
        let row: Option<(String, i64, String, String)> = tx
            .query_row(
                "SELECT state, pid, started_at, detail FROM publishing WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
                params![channel, platform, video_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let claim = match row {
            None => {
                if let Some(limit) = daily_limit {
                    let others: i64 = tx.query_row(
                        "SELECT COUNT(DISTINCT video_id) FROM (
                            SELECT video_id FROM posts WHERE channel = ?1 AND test = 0 AND datetime(posted_at) > datetime('now','-1 day')
                            UNION ALL
                            SELECT video_id FROM publishing WHERE channel = ?1 AND test = 0 AND datetime(started_at) > datetime('now','-1 day')
                         ) WHERE video_id != ?2",
                        params![channel, video_id],
                        |r| r.get(0),
                    )?;
                    let same: i64 = tx.query_row(
                        "SELECT (SELECT COUNT(*) FROM posts WHERE channel = ?1 AND video_id = ?2)
                              + (SELECT COUNT(*) FROM publishing WHERE channel = ?1 AND video_id = ?2)",
                        params![channel, video_id],
                        |r| r.get(0),
                    )?;
                    if same == 0 && others >= limit {
                        return Ok(Claim::LimitReached);
                    }
                }
                tx.execute(
                    "INSERT INTO publishing (channel, platform, video_id, state, pid, started_at, caption, test) VALUES (?1,?2,?3,'publishing',?4,?5,?6,?7)",
                    params![channel, platform, video_id, me, now(), caption, crate::publish::test_post_for(channel) as i64],
                )?;
                Claim::Fresh
            }
            Some((state, pid, since, _)) if state == "publishing" && publisher_alive(pid as u32, &since) => {
                return Ok(Claim::Busy { pid: pid as u32, since });
            }
            Some((_, _, since, detail)) => {
                // dono morto ou tentativa sem confirmacao: assume, mas quem chama confere o perfil antes
                tx.execute(
                    "UPDATE publishing SET state = 'publishing', pid = ?4 WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
                    params![channel, platform, video_id, me],
                )?;
                Claim::Unconfirmed { since, detail }
            }
        };
        tx.commit()?;
        Ok(claim)
    }

    /// Upload terminou sem saber se publicou: fica para conferir o perfil na proxima tentativa.
    pub fn mark_unconfirmed(&self, channel: &str, platform: &str, video_id: &str, detail: &str) -> Result<()> {
        self.db().execute(
            "UPDATE publishing SET state = 'unconfirmed', detail = ?4 WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
            params![channel, platform, video_id, detail],
        )?;
        Ok(())
    }

    /// Falha definitiva (nada saiu): libera para tentar de novo do zero.
    pub fn release_publish(&self, channel: &str, platform: &str, video_id: &str) -> Result<()> {
        self.db().execute(
            "DELETE FROM publishing WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
            params![channel, platform, video_id],
        )?;
        Ok(())
    }

    /// Publicou (confirmado): grava o post e tira a reserva na mesma transacao.
    pub fn finish_publish(&self, channel: &str, platform: &str, video_id: &str, url: &str, caption: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO posts (channel, platform, video_id, url, caption, posted_at, test) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![channel, platform, video_id, url, caption, now(), crate::publish::test_post_for(channel) as i64],
        )?;
        tx.execute(
            "DELETE FROM publishing WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
            params![channel, platform, video_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Publicacoes reservadas que nao estao rodando agora (app morreu, sem confirmacao).
    pub fn unconfirmed_publishes(&self, channel: Option<&str>) -> Result<Vec<Pending>> {
        let db = self.db();
        let mut st = db.prepare(
            "SELECT channel, platform, video_id, state, pid, started_at, caption, detail FROM publishing
             WHERE ?1 IS NULL OR channel = ?1 ORDER BY started_at",
        )?;
        let rows = st
            .query_map(params![channel], |r| {
                Ok(Pending {
                    channel: r.get(0)?,
                    platform: r.get(1)?,
                    video_id: r.get(2)?,
                    state: r.get(3)?,
                    pid: r.get::<_, i64>(4)? as u32,
                    started_at: r.get(5)?,
                    caption: r.get(6)?,
                    detail: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|p| !(p.state == "publishing" && publisher_alive(p.pid, &p.started_at))).collect())
    }

    /// Ja postou esse video nessa plataforma? (pra nao repetir quando uma rede falhou e a outra nao)
    pub fn posted_on(&self, channel: &str, platform: &str, video_id: &str) -> Result<bool> {
        let n: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM posts WHERE channel = ?1 AND platform = ?2 AND video_id = ?3",
            params![channel, platform, video_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Tentativas ja gastas no horario `idx` do dia `day` (YYYY-MM-DD).
    pub fn slot_attempts(&self, channel: &str, day: &str, idx: usize) -> Result<u32> {
        Ok(self
            .db()
            .query_row(
                "SELECT n FROM slot_attempts WHERE channel = ?1 AND day = ?2 AND idx = ?3",
                params![channel, day, idx as i64],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u32)
    }

    /// Soma uma tentativa (`to = None`) ou fixa o valor. Dias com mais de 2 dias saem.
    pub fn bump_slot_attempts(&self, channel: &str, day: &str, idx: usize, to: Option<u32>) -> Result<()> {
        let db = self.db();
        db.execute("DELETE FROM slot_attempts WHERE day < date(?1, '-2 days')", params![day])?;
        match to {
            Some(n) => db.execute(
                "INSERT INTO slot_attempts (channel, day, idx, n) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(channel, day, idx) DO UPDATE SET n = excluded.n",
                params![channel, day, idx as i64, n as i64],
            )?,
            None => db.execute(
                "INSERT INTO slot_attempts (channel, day, idx, n) VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(channel, day, idx) DO UPDATE SET n = n + 1",
                params![channel, day, idx as i64],
            )?,
        };
        Ok(())
    }

    /// Hora (rfc3339) do ultimo post do canal, contando upload em andamento/sem confirmacao.
    pub fn last_activity_at(&self, channel: &str) -> Result<Option<String>> {
        Ok(self.db().query_row(
            "SELECT MAX(t) FROM (
                SELECT datetime(posted_at) AS t FROM posts WHERE channel = ?1 AND test = 0
                UNION ALL
                SELECT datetime(started_at) FROM publishing WHERE channel = ?1 AND test = 0
             )",
            params![channel],
            |r| r.get::<_, Option<String>>(0),
        )?)
    }

    pub fn last_post_at(&self, channel: &str) -> Result<Option<String>> {
        let v: Option<String> = self.db().query_row(
                "SELECT posted_at FROM posts WHERE channel = ?1 ORDER BY posted_at DESC LIMIT 1",
                params![channel],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v)
    }

    /// Captions dos ultimos N posts — contexto anti-repeticao do ideate.
    /// Criticas das ultimas reprovacoes — contexto corretivo do proximo ideate.
    pub fn recent_review_problems(&self, channel: &str, limit: i64) -> Result<Vec<String>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT detail FROM runs WHERE channel = ?1 AND step = 'review' AND status = 'reprovado'
             ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![channel, limit], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn recent_captions(&self, channel: &str, limit: i64) -> Result<Vec<String>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT caption FROM posts WHERE channel = ?1 ORDER BY posted_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![channel, limit], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    // ------------------------------------------------------------ agentes

    pub fn record_agent_call(&self, c: &AgentCall) -> Result<()> {
        self.db()
            .prepare_cached(
                "INSERT OR IGNORE INTO agent_calls (channel, task, model, effort, input, output, cache_read, cache_write, cost, turns, ms, ok, source, session, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            )?
            .execute(params![
                c.channel, c.task, c.model, c.effort, c.input, c.output, c.cache_read, c.cache_write,
                c.cost, c.turns, c.ms, c.ok as i32, c.source, c.session, c.at
            ])?;
        Ok(())
    }

    /// Soma de uso agrupada por `group` (channel | task | model | pair = canal x tarefa) desde `since` (rfc3339).
    /// Por tarefa: (tarefa, chamadas antes, tokens antes, chamadas depois, tokens depois) do corte `cut`.
    /// Sem as conversas interativas.
    pub fn usage_split(&self, cut: &str) -> Result<Vec<(String, i64, i64, i64, i64)>> {
        let db = self.db();
        let mut stmt = db.prepare_cached(
            "SELECT task,
                    SUM(at <  ?1), SUM(CASE WHEN at <  ?1 THEN input + output + cache_read + cache_write ELSE 0 END),
                    SUM(at >= ?1), SUM(CASE WHEN at >= ?1 THEN input + output + cache_read + cache_write ELSE 0 END)
             FROM agent_calls WHERE task != 'interativo' GROUP BY task ORDER BY task",
        )?;
        let rows = stmt
            .query_map(params![cut], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn usage_by(&self, group: &str, since: &str) -> Result<Vec<UsageRow>> {
        let cols = match group {
            "channel" => "channel, ''",
            "task" => "'', task",
            "model" => "'', model",
            _ => "channel, task",
        };
        let sql = format!(
            "SELECT {cols}, COUNT(*), SUM(input), SUM(output), SUM(cache_read), SUM(cache_write), SUM(cost), SUM(turns), SUM(ms)
             FROM agent_calls WHERE at >= ?1 GROUP BY 1, 2 ORDER BY SUM(input + output + cache_read + cache_write) DESC"
        );
        let db = self.db();
        let mut stmt = db.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(params![since], |r| {
                Ok(UsageRow {
                    channel: r.get(0)?,
                    task: r.get(1)?,
                    calls: r.get(2)?,
                    input: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    output: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    cache_read: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    cache_write: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    cost: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                    turns: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                    ms: r.get::<_, Option<i64>>(9)?.unwrap_or(0),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Uso por dia (yyyy-mm-dd UTC): (dia, tokens novos, leitura de cache).
    pub fn usage_daily(&self, since: &str) -> Result<Vec<(String, i64, i64)>> {
        let db = self.db();
        let mut stmt = db.prepare_cached(
            "SELECT substr(at, 1, 10), SUM(input + output + cache_write), SUM(cache_read)
             FROM agent_calls WHERE at >= ?1 GROUP BY 1 ORDER BY 1",
        )?;
        let rows = stmt
            .query_map(params![since], |r| {
                Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0)))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn agent_calls_count(&self, source: &str) -> Result<i64> {
        Ok(self.db().query_row("SELECT COUNT(*) FROM agent_calls WHERE source = ?1", params![source], |r| r.get(0))?)
    }

    // ------------------------------------------------------------ eventos

    pub fn insert_events(&self, evs: &[crate::events::Event]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO events (run_id, channel, step, action, tool, status, progress, detail, at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )?;
            for e in evs {
                st.execute(params![e.run_id, e.channel, e.step, e.action, e.tool, e.status, e.progress, e.detail, e.at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Fecha os runs que ficaram abertos (app fechado ou derrubado no meio): a etapa que estava
    /// rodando vira erro "interrompido" e o run ganha `ended_at`. Chamar ao abrir o app.
    /// Apaga todo o historico de um canal excluido (Ao vivo, Consumo, videos, posts, referencias).
    pub fn forget_channel(&self, channel: &str) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut n = 0;
        for t in ["events", "runs", "agent_calls", "videos", "posts", "ref_videos", "reviews", "publishing", "sessions"] {
            n += tx.execute(&format!("DELETE FROM {t} WHERE channel = ?1"), params![channel])?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Canal renomeado: o historico (Ao vivo, Consumo, videos, posts, referencias) vai junto.
    pub fn rename_channel(&self, old: &str, new: &str) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut n = 0;
        for t in ["events", "runs", "agent_calls", "videos", "posts", "ref_videos", "reviews", "publishing", "sessions"] {
            n += tx.execute(&format!("UPDATE {t} SET channel = ?2 WHERE channel = ?1"), params![old, new])?;
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn close_open_runs(&self) -> Result<usize> {
        let now = now();
        let mut db = self.db();
        let tx = db.transaction()?;
        let runs: Vec<(String, String)> = {
            let mut st = tx.prepare(
                "SELECT run_id, channel FROM runs WHERE step = 'run' AND ended_at IS NULL AND status = 'start' AND run_id != ''",
            )?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            v
        };
        for (run_id, channel) in &runs {
            let open: Vec<(String, String, String)> = {
                let mut st = tx.prepare(
                    "SELECT step, action, tool FROM events WHERE id IN
                       (SELECT MAX(id) FROM events WHERE run_id = ?1 GROUP BY step)
                     AND status IN ('start', 'progress')
                     ORDER BY CASE WHEN step IN ('agent', 'browser') THEN 0 ELSE 1 END, id",
                )?;
                let v = st.query_map(params![run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
                v
            };
            for (step, action, tool) in open {
                tx.execute(
                    "INSERT INTO events (run_id, channel, step, action, tool, status, progress, detail, at)
                     VALUES (?1,?2,?3,?4,?5,'error',NULL,'interrompido',?6)",
                    params![run_id, channel, step, action, tool, now],
                )?;
            }
            tx.execute(
                "UPDATE runs SET status = 'interrompido', detail = 'app fechado no meio', ended_at = ?2 WHERE run_id = ?1 AND step = 'run'",
                params![run_id, now],
            )?;
        }
        // etapas soltas, fora de run aberto (live assistida, "live" sem run): ficavam "trabalhando" no Ao vivo pra
        // sempre depois de fechar o app no meio. Ao abrir nada esta rodando ainda, entao toda etapa aberta e velha.
        let cutoff = days_ago(3);
        let stale: Vec<(String, String, String, String, String)> = {
            let mut st = tx.prepare(
                "SELECT run_id, channel, step, action, tool FROM events WHERE id IN
                   (SELECT MAX(id) FROM events WHERE at >= ?1 GROUP BY run_id, channel, step)
                 AND status IN ('start', 'progress')",
            )?;
            let v = st.query_map(params![cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
            v
        };
        for (run_id, channel, step, action, tool) in &stale {
            tx.execute(
                "INSERT INTO events (run_id, channel, step, action, tool, status, progress, detail, at)
                 VALUES (?1,?2,?3,?4,?5,'error',NULL,'interrompido',?6)",
                params![run_id, channel, step, action, tool, now],
            )?;
        }
        tx.commit()?;
        Ok(runs.len())
    }

    pub fn recent_events(&self, channel: Option<&str>, limit: i64) -> Result<Vec<crate::events::Event>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT run_id, channel, step, action, tool, status, progress, detail, at FROM events
             WHERE (?1 IS NULL OR channel = ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let mut rows = st
            .query_map(params![channel, limit], |r| {
                Ok(crate::events::Event {
                    run_id: r.get(0)?,
                    channel: r.get(1)?,
                    step: r.get(2)?,
                    action: r.get(3)?,
                    tool: r.get(4)?,
                    status: r.get(5)?,
                    progress: r.get(6)?,
                    detail: r.get(7)?,
                    at: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.reverse();
        Ok(rows)
    }

    // ------------------------------------------------------------ agenda

    /// Ultima vez que uma etapa foi registrada no canal.
    pub fn last_step_at(&self, channel: &str, step: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT at FROM runs WHERE channel = ?1 AND step = ?2 ORDER BY at DESC LIMIT 1",
                params![channel, step],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Ultimo registro do canal: (etapa, status, detalhe, quando).
    pub fn last_run(&self, channel: &str) -> Result<Option<(String, String, String, String)>> {
        Ok(self
            .db()
            .query_row(
                "SELECT step, status, detail, at FROM runs WHERE channel = ?1 ORDER BY id DESC LIMIT 1",
                params![channel],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }

    /// Posts feitos desde `since` (rfc3339).
    /// Videos postados desde `since` (RFC 3339), contando os que estao no meio da publicacao ou sem confirmacao.
    /// Antes: COUNT(*) das linhas de `posts`, que tem UMA LINHA POR REDE (TikTok + YouTube = 2 e o 2o horario do
    /// dia era pulado), e sem as reservas (reboot no meio do upload: o mesmo horario fazia e postava outro video).
    pub fn posts_since(&self, channel: &str, since: &str) -> Result<i64> {
        Ok(self.db().query_row(
            "SELECT COUNT(DISTINCT video_id) FROM (
                SELECT video_id FROM posts WHERE channel = ?1 AND test = 0 AND datetime(posted_at) >= datetime(?2)
                UNION ALL
                SELECT video_id FROM publishing WHERE channel = ?1 AND test = 0 AND datetime(started_at) >= datetime(?2)
             )",
            params![channel, since],
            |r| r.get(0),
        )?)
    }

    /// Videos aprovados e nota media.
    pub fn library_stats(&self, channel: &str) -> Result<(i64, Option<f64>)> {
        Ok(self.db().query_row(
            "SELECT COUNT(*), AVG(score) FROM videos WHERE channel = ?1 AND kept = 1",
            params![channel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentCall {
    pub channel: String,
    pub task: String,
    pub model: String,
    pub effort: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost: f64,
    pub turns: i64,
    pub ms: i64,
    pub ok: bool,
    /// tkd (chamada nova) | import (historico do ~/.claude)
    pub source: String,
    /// session_id do claude (evita importar duas vezes)
    pub session: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UsageRow {
    pub channel: String,
    pub task: String,
    pub calls: i64,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost: f64,
    pub turns: i64,
    pub ms: i64,
}

/// rfc3339 de N dias atras (comparavel com as colunas `at`).
/// Resultado de `claim_publish`.
#[derive(Debug, Clone, PartialEq)]
pub enum Claim {
    /// reservado agora: pode subir
    Fresh,
    /// ja esta no banco como postado
    AlreadyPosted,
    /// o canal ja bateu o limite de posts do dia (conferido na mesma transacao da reserva)
    LimitReached,
    /// outro processo (ou outra task) esta publicando esse video agora
    Busy { pid: u32, since: String },
    /// tentativa anterior morreu sem confirmar: conferir o perfil antes de reenviar
    Unconfirmed { since: String, detail: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Pending {
    pub channel: String,
    pub platform: String,
    pub video_id: String,
    pub state: String,
    pub pid: u32,
    pub started_at: String,
    pub caption: String,
    pub detail: String,
}

/// Teto de uma publicacao (o navegador tem 30 min + folga): passou disso o dono travou.
const PUBLISH_LEASE_MIN: i64 = 40;

/// O processo que reservou a publicacao ainda esta vivo e dentro do prazo?
fn publisher_alive(pid: u32, since: &str) -> bool {
    let fresh = DateTime::parse_from_rfc3339(since)
        .map(|t| Utc::now().signed_duration_since(t) < chrono::Duration::minutes(PUBLISH_LEASE_MIN))
        .unwrap_or(false);
    fresh && crate::proc::pid_alive(pid)
}

pub fn days_ago(n: i64) -> String {
    (Utc::now() - chrono::Duration::days(n)).to_rfc3339()
}


pub fn now() -> String {
    let t: DateTime<Utc> = Utc::now();
    t.to_rfc3339()
}

#[cfg(test)]
mod close_runs_tests {
    use super::*;

    #[test]
    fn forgets_only_deleted_channel() {
        let s = Store::memory().unwrap();
        s.log("a", "ideate", "ok", "").unwrap();
        s.log("b", "ideate", "ok", "").unwrap();
        assert_eq!(s.forget_channel("a").unwrap(), 1);
        let n: i64 = s.db().query_row("SELECT COUNT(*) FROM runs WHERE channel = 'b'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    /// Post de teste (botao do app) fica no historico e no "ja postado", mas nao gasta o dia: limite, horarios
    /// e intervalo minimo contam so os posts de verdade. E so do canal em teste.
    #[test]
    fn test_posts_do_not_count() {
        let s = Store::memory().unwrap();
        {
            let _t = crate::publish::TestPost::begin("teste_a").unwrap();
            assert!(crate::publish::TestPost::begin("teste_a").is_none(), "um post de teste por vez");
            assert!(!crate::publish::test_post_for("teste_b"), "o outro canal continua publico");
            s.record_post("teste_a", "tiktok", "v1", "", "").unwrap();
        }
        assert!(!crate::publish::test_post_for("teste_a"), "desliga ao terminar");
        assert!(s.posted_on("teste_a", "tiktok", "v1").unwrap(), "ainda conta como ja postado");
        assert_eq!(s.posts_last_24h("teste_a").unwrap(), 0);
        assert_eq!(s.posts_since("teste_a", "2000-01-01T00:00:00Z").unwrap(), 0);
        assert!(s.last_activity_at("teste_a").unwrap().is_none());
        s.record_post("teste_a", "tiktok", "v2", "", "").unwrap();
        assert_eq!(s.posts_last_24h("teste_a").unwrap(), 1);
    }

    #[test]
    fn daily_limit_counts_video_and_date() {
        let s = Store::memory().unwrap();
        s.record_post("a", "tiktok", "v1", "", "").unwrap();
        s.record_post("a", "youtube", "v1", "", "").unwrap();
        assert_eq!(s.posts_last_24h("a").unwrap(), 1);
        assert!(s.posted_on("a", "youtube", "v1").unwrap());
        assert!(!s.posted_on("a", "youtube", "v2").unwrap());
        // post de 2 dias atras no mesmo formato RFC 3339 nao conta
        s.db()
            .execute(
                "INSERT INTO posts (channel, platform, video_id, posted_at) VALUES ('a','tiktok','v0',?1)",
                params![(Utc::now() - chrono::Duration::hours(30)).to_rfc3339()],
            )
            .unwrap();
        assert_eq!(s.posts_last_24h("a").unwrap(), 1);
    }

    /// App e CLI ao mesmo tempo, videos diferentes, limite 1/dia: so um reserva (duas conexoes no mesmo arquivo).
    #[test]
    fn daily_limit_across_processes() {
        let p = std::env::temp_dir().join(format!("tkd-limite-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let open = || {
            let c = Connection::open(&p).unwrap();
            c.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
            let s = Store { conn: Mutex::new(c) };
            s.migrate().unwrap();
            s
        };
        let (app, cli) = (open(), open());
        assert_eq!(app.claim_publish_within("a", "tiktok", "v1", "x", Some(1)).unwrap(), Claim::Fresh);
        assert_eq!(cli.claim_publish_within("a", "tiktok", "v2", "y", Some(1)).unwrap(), Claim::LimitReached);
        // a outra rede do mesmo video nao e post novo
        assert_eq!(cli.claim_publish_within("a", "youtube", "v1", "x", Some(1)).unwrap(), Claim::Fresh);
        drop((app, cli));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn session_expired_until_new_login() {
        let s = Store::memory().unwrap();
        assert!(!s.session_expired("a", "tiktok").unwrap());
        s.set_session("a", "tiktok", false).unwrap();
        assert!(s.session_expired("a", "tiktok").unwrap());
        assert!(!s.session_expired("a", "youtube").unwrap());
        s.set_session("a", "tiktok", true).unwrap();
        assert!(!s.session_expired("a", "tiktok").unwrap());
    }

    #[test]
    fn publish_reservation() {
        let s = Store::memory().unwrap();
        assert_eq!(s.claim_publish("a", "tiktok", "v1", "oi").unwrap(), Claim::Fresh);
        // mesma maquina, dono vivo (este processo): segunda tentativa nao passa
        assert!(matches!(s.claim_publish("a", "tiktok", "v1", "oi").unwrap(), Claim::Busy { .. }));
        // outra rede do mesmo video e independente
        assert_eq!(s.claim_publish("a", "youtube", "v1", "oi").unwrap(), Claim::Fresh);
        // reserva conta no limite diario (pode estar no ar)
        assert_eq!(s.posts_last_24h("a").unwrap(), 1);
        assert!(s.unconfirmed_publishes(None).unwrap().is_empty());

        // app morreu entre o upload e o record_post: sem confirmacao
        s.mark_unconfirmed("a", "tiktok", "v1", "navegador morreu").unwrap();
        assert_eq!(s.unconfirmed_publishes(Some("a")).unwrap().len(), 1);
        match s.claim_publish("a", "tiktok", "v1", "oi").unwrap() {
            Claim::Unconfirmed { detail, .. } => assert_eq!(detail, "navegador morreu"),
            c => panic!("{c:?}"),
        }
        s.finish_publish("a", "tiktok", "v1", "https://x/video/1", "oi").unwrap();
        assert_eq!(s.claim_publish("a", "tiktok", "v1", "oi").unwrap(), Claim::AlreadyPosted);

        // dono morto (pid que nao existe) com a reserva "publishing": vira sem confirmacao
        s.db()
            .execute("UPDATE publishing SET pid = 4000000000 WHERE video_id = 'v1' AND platform = 'youtube'", [])
            .unwrap();
        assert!(matches!(s.claim_publish("a", "youtube", "v1", "oi").unwrap(), Claim::Unconfirmed { .. }));
        s.release_publish("a", "youtube", "v1").unwrap();
        assert_eq!(s.claim_publish("a", "youtube", "v1", "oi").unwrap(), Claim::Fresh);
    }

    #[test]
    fn renames_history() {
        let s = Store::memory().unwrap();
        s.log("velho", "ideate", "ok", "").unwrap();
        s.record_post("velho", "tiktok", "v1", "", "").unwrap();
        s.log("outro", "ideate", "ok", "").unwrap();
        assert_eq!(s.rename_channel("velho", "novo").unwrap(), 2);
        assert_eq!(s.posts_last_24h("novo").unwrap(), 1);
        let n: i64 = s.db().query_row("SELECT COUNT(*) FROM runs WHERE channel = 'outro'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn open_run_becomes_interrupted() {
        let s = Store::memory().unwrap();
        {
            let db = s.db();
            db.execute("INSERT INTO runs (channel, step, status, detail, at, run_id) VALUES ('_t','run','start','','x','r1')", []).unwrap();
            db.execute("INSERT INTO runs (channel, step, status, detail, at, run_id, ended_at) VALUES ('_t','run','ok','','x','r0','y')", []).unwrap();
            for (step, st) in [("ideate", "ok"), ("agent", "start"), ("agent", "ok"), ("review", "progress"), ("agent", "progress")] {
                db.execute(
                    "INSERT INTO events (run_id, channel, step, action, tool, status, progress, detail, at) VALUES ('r1','_t',?1,'a','claude',?2,NULL,'','x')",
                    params![step, st],
                )
                .unwrap();
            }
        }
        assert_eq!(s.close_open_runs().unwrap(), 1);
        let db = s.db();
        let (st, end): (String, Option<String>) =
            db.query_row("SELECT status, ended_at FROM runs WHERE run_id='r1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(st, "interrompido");
        assert!(end.is_some());
        let mut q = db.prepare("SELECT step FROM events WHERE status='error' ORDER BY step").unwrap();
        let steps: Vec<String> = q.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(steps, ["agent", "review"]);
        drop(q);
        drop(db);
        assert_eq!(s.close_open_runs().unwrap(), 0, "idempotente");
    }

    #[test]
    fn stray_live_becomes_interrupted() {
        // live assistida nao tem run aberto proprio: ficava "trabalhando" pra sempre depois de fechar o app
        let s = Store::memory().unwrap();
        let t = now();
        {
            let db = s.db();
            for (run, ch, step, st) in [("", "live", "live", "progress"), ("r9", "c", "live:streamer_a", "start"), ("r9", "c", "live:x", "ok")] {
                db.execute(
                    "INSERT INTO events (run_id, channel, step, action, tool, status, progress, detail, at) VALUES (?1,?2,?3,'a','python',?4,NULL,'',?5)",
                    params![run, ch, step, st, t],
                )
                .unwrap();
            }
        }
        s.close_open_runs().unwrap();
        let db = s.db();
        let mut q = db.prepare("SELECT step FROM events WHERE status='error' AND detail='interrompido' ORDER BY step").unwrap();
        let steps: Vec<String> = q.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(steps, ["live", "live:streamer_a"]);
    }
}
