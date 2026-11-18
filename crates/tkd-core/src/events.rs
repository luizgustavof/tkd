//! Eventos ao vivo: o que cada agente esta fazendo agora.
//!
//! `step(canal, etapa)` devolve um `Span` (guard RAII) que emite inicio, progresso e fim.
//! Se o Span cair sem `ok()`/`fail()` (erro com `?`, cancelamento), o `Drop` emite erro.
//! Tudo vai para um `broadcast` (app ao vivo) e e gravado em lote no SQLite (tabela events).

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Event {
    pub run_id: String,
    pub channel: String,
    /// scout | study | ideate | render | review | publish | footage | recreate | live | login | agent
    pub step: String,
    /// Frase curta do que esta acontecendo ("Assistindo video 4 de 9")
    pub action: String,
    /// claude | edge | remotion | ffmpeg | python
    pub tool: String,
    /// start | progress | ok | error | info
    pub status: String,
    /// 0..100
    pub progress: Option<f64>,
    pub detail: String,
    pub at: String,
}

struct Bus {
    tx: broadcast::Sender<Event>,
    persist: Mutex<Option<mpsc::Sender<Event>>>,
    /// thread que grava; `flush` espera ela terminar
    writer: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// run_id atual por canal (agrupa as etapas de um video)
    runs: Mutex<HashMap<String, String>>,
    /// ultimo evento de cada (canal, etapa) — estado inicial do Ao vivo
    last: Mutex<HashMap<(String, String), Event>>,
}

fn bus() -> &'static Bus {
    static BUS: OnceLock<Bus> = OnceLock::new();
    BUS.get_or_init(|| {
        let (tx, _) = broadcast::channel(1024);
        Bus { tx, persist: Mutex::new(None), writer: Mutex::new(None), runs: Mutex::new(HashMap::new()), last: Mutex::new(HashMap::new()) }
    })
}

/// Liga a gravacao dos eventos no SQLite (thread proprio, lotes a cada 400 ms).
/// Sem isso os eventos so vao para o broadcast (CLI).
pub fn enable_persistence() {
    let mut g = bus().persist.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Event>();
    *g = Some(tx);
    let h = std::thread::Builder::new().name("tkd-events".into()).spawn(move || writer(rx)).ok();
    *bus().writer.lock().unwrap_or_else(|e| e.into_inner()) = h;
}

/// Grava o que falta e desliga a gravacao. Chamar antes do processo sair (a CLI saia
/// antes do ultimo lote e perdia o fim do run).
pub fn flush() {
    bus().persist.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(h) = bus().writer.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = h.join();
    }
}

/// Eventos guardados em memoria enquanto o banco nao aceita (travado pelo OneDrive, outro processo...).
const MAX_PENDING: usize = 20_000;

fn writer(rx: mpsc::Receiver<Event>) {
    // banco travado na abertura nao pode desligar a gravacao do processo inteiro (antes: `return` e todos os
    // eventos dali em diante se perdiam). Tenta de novo a cada lote; o que nao gravou fica no buffer.
    let mut store: Option<crate::state::Store> = None;
    let mut warned = false;
    let mut buf: Vec<Event> = Vec::new();
    let mut done = false;
    let mut done_at: Option<std::time::Instant> = None;
    // a poda de 7 dias so rodava ao abrir o banco: app aberto por semanas nunca podava
    let mut pruned = std::time::Instant::now();
    loop {
        if store.is_none() {
            match crate::state::Store::open() {
                Ok(s) => {
                    if warned {
                        tracing::info!("banco de volta: gravando {} evento(s) guardados", buf.len());
                    }
                    store = Some(s);
                }
                Err(e) if !warned => {
                    tracing::warn!("eventos sem banco por enquanto ({e:#}); guardando em memoria");
                    warned = true;
                }
                Err(_) => {}
            }
        }
        if let Some(st) = &store {
            if pruned.elapsed() > Duration::from_secs(3600) {
                pruned = std::time::Instant::now();
                if let Err(e) = st.prune_events() {
                    tracing::warn!("podando eventos: {e}");
                }
            }
        }
        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(e) => {
                // progresso repetido da mesma etapa: guarda so o mais novo
                if e.status == "progress" {
                    if let Some(prev) = buf
                        .iter_mut()
                        .rev()
                        .find(|p| p.status == "progress" && p.channel == e.channel && p.step == e.step)
                    {
                        *prev = e;
                        continue;
                    }
                }
                buf.push(e);
                if buf.len() < 200 {
                    continue;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => done = true,
        }
        if !buf.is_empty() {
            match store.as_ref().map(|s| s.insert_events(&buf)) {
                Some(Ok(())) => buf.clear(),
                Some(Err(e)) => {
                    // lote que falhou fica para a proxima (antes era jogado fora)
                    if !warned {
                        tracing::warn!("gravando eventos: {e}; tento de novo");
                        warned = true;
                    }
                }
                None => {}
            }
            if buf.len() > MAX_PENDING {
                let n = buf.len() - MAX_PENDING;
                buf.drain(..n);
                tracing::warn!("banco fora ha muito tempo: {n} evento(s) mais velhos descartados");
            }
        }
        if done {
            // saindo: insiste um pouco (ate 30 s) antes de desistir do que falta
            let t = *done_at.get_or_insert_with(std::time::Instant::now);
            if buf.is_empty() || t.elapsed() > Duration::from_secs(30) {
                if !buf.is_empty() {
                    tracing::error!("{} evento(s) nao gravados: banco travado", buf.len());
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

pub fn subscribe() -> broadcast::Receiver<Event> {
    bus().tx.subscribe()
}

/// Ultimo evento de cada etapa de cada canal.
pub fn snapshot() -> Vec<Event> {
    let g = bus().last.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<Event> = g.values().cloned().collect();
    v.sort_by(|a, b| a.at.cmp(&b.at));
    v
}

pub fn emit(e: Event) {
    let b = bus();
    if let Ok(mut g) = b.last.lock() {
        g.insert((e.channel.clone(), e.step.clone()), e.clone());
    }
    if let Some(tx) = b.persist.lock().ok().and_then(|g| g.clone()) {
        let _ = tx.send(e.clone());
    }
    let _ = b.tx.send(e);
}

/// Comeca um "video" novo no canal: as etapas seguintes ganham o mesmo run_id.
pub fn begin_run(channel: &str) -> String {
    let id = format!("{channel}-{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"));
    if let Ok(mut g) = bus().runs.lock() {
        g.insert(channel.to_string(), id.clone());
    }
    if let Ok(mut g) = bus().last.lock() {
        // live assistida continua rodando por cima dos videos: nao some do estado inicial do Ao vivo
        g.retain(|(c, s), _| c != channel || s.starts_with("live:"));
    }
    id
}

fn run_of(channel: &str) -> String {
    bus().runs.lock().ok().and_then(|g| g.get(channel).cloned()).unwrap_or_default()
}

/// Uma etapa em andamento. Emite `start` ao nascer e `error` se cair sem fechar.
pub struct Span {
    channel: String,
    step: String,
    run_id: String,
    tool: Mutex<String>,
    action: Mutex<String>,
    last_progress: Mutex<Option<Instant>>,
    done: Mutex<bool>,
}

pub fn step(channel: &str, step: &str) -> Span {
    let s = Span {
        channel: channel.to_string(),
        step: step.to_string(),
        run_id: run_of(channel),
        tool: Mutex::new(String::new()),
        action: Mutex::new(String::new()),
        last_progress: Mutex::new(None),
        done: Mutex::new(false),
    };
    s.send("start", None, "");
    s
}

impl Span {
    fn send(&self, status: &str, progress: Option<f64>, detail: &str) {
        emit(Event {
            run_id: self.run_id.clone(),
            channel: self.channel.clone(),
            step: self.step.clone(),
            action: self.action.lock().map(|g| g.clone()).unwrap_or_default(),
            tool: self.tool.lock().map(|g| g.clone()).unwrap_or_default(),
            status: status.to_string(),
            progress,
            detail: detail.to_string(),
            at: crate::state::now(),
        });
    }

    /// Troca a frase do que esta acontecendo e a ferramenta em uso.
    pub fn action(&self, action: &str, tool: &str) -> &Self {
        if let Ok(mut g) = self.action.lock() {
            *g = action.to_string();
        }
        if let Ok(mut g) = self.tool.lock() {
            *g = tool.to_string();
        }
        self.send("progress", None, "");
        self
    }

    /// Progresso 0..100 (no maximo ~8 por segundo; 100 sempre passa).
    pub fn progress(&self, p: f64) -> &Self {
        let now = Instant::now();
        if let Ok(mut g) = self.last_progress.lock() {
            if p < 100.0 && g.is_some_and(|t| now.duration_since(t) < Duration::from_millis(120)) {
                return self;
            }
            *g = Some(now);
        }
        self.send("progress", Some(p.clamp(0.0, 100.0)), "");
        self
    }

    /// Linha de log (vai para o inspetor do Ao vivo).
    pub fn info(&self, detail: &str) -> &Self {
        self.send("info", None, detail);
        self
    }

    pub fn ok(&self, detail: &str) {
        if let Ok(mut d) = self.done.lock() {
            *d = true;
        }
        self.send("ok", Some(100.0), detail);
    }

    pub fn fail(&self, err: &str) {
        if let Ok(mut d) = self.done.lock() {
            *d = true;
        }
        self.send("error", None, err);
    }

    /// Fecha com ok/erro conforme o resultado e devolve o proprio resultado.
    pub fn finish<T>(&self, r: anyhow::Result<T>, ok_detail: impl FnOnce(&T) -> String) -> anyhow::Result<T> {
        match &r {
            Ok(v) => self.ok(&ok_detail(v)),
            Err(e) => self.fail(&format!("{e:#}")),
        }
        r
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.done.lock().map(|g| *g).unwrap_or(true) {
            self.send("error", None, "interrompido");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_cai_vira_erro() {
        let mut rx = subscribe();
        {
            let s = step("_t", "render");
            s.action("Renderizando", "remotion").progress(62.0);
        }
        let mut st = vec![];
        while let Ok(e) = rx.try_recv() {
            if e.channel == "_t" {
                st.push(e.status);
            }
        }
        assert_eq!(st.first().map(String::as_str), Some("start"));
        assert_eq!(st.last().map(String::as_str), Some("error"));
    }
}
