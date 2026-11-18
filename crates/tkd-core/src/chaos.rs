//! Macaco do caos (estilo Chaos Monkey): injeta falha de proposito para provar que o app falha limpo.
//!
//! Desligado por padrao. Liga com `TKD_CHAOS`, regras separadas por virgula `tipo:alvo[:arg[:arg2]]`:
//!
//! - `kill:<bin>:<prob>[:<max>]` mata a ARVORE do filho num instante aleatorio ate `<max>` (padrao 10s)
//! - `crash:<bin>:<prob>[:<max>]` mata so o processo direto (os netos ficam por conta do `proc::Tree`)
//! - `fail:<bin>:<prob>` o filho "sai com erro" sem rodar
//! - `block:<bin>:<prob>` igual ao fail, com a mensagem de bloqueio do YouTube (429 / "not a bot")
//! - `delay:<bin>:<tempo>` atrasa a resposta do filho (PC lento / rede lenta)
//! - `hang:<bin>[:<prob>]` o filho nunca termina (testa os timeouts)
//!
//! Ganchos fora dos processos (alvo fixo):
//! - `fail:write:<prob>` / `full:write:<prob>`: `config::write_atomic` falha (erro comum / disco cheio)
//! - `empty:claude:<prob>`, `truncate:claude:<prob>`, `ratelimit:claude:<prob>`: resposta do `claude -p`
//!
//! `<bin>` casa com o nome do executavel (`ffmpeg`, `npx`, `node`, `python`, `claude`, `yt-dlp`, `curl`)
//! ou com o script que ele roda (`upload`, `speech`, `eval_video`, `live`); `*` casa com todos.
//! `TKD_CHAOS_SEED` fixa a semente para repetir um experimento que achou bug.
//! Cada injecao vai para o log e para `out/chaos/injections.jsonl`.

use rand::{rngs::StdRng, Rng, SeedableRng};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Kill,
    Crash,
    Fail,
    Block,
    Delay,
    Hang,
    Full,
    Empty,
    Truncate,
    RateLimit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub kind: Kind,
    pub target: String,
    pub prob: f64,
    /// `delay`: quanto atrasar; `kill`/`crash`: instante maximo da morte
    pub dur: Duration,
}

pub struct Monkey {
    pub rules: Vec<Rule>,
    pub seed: u64,
    rng: Mutex<StdRng>,
}

/// O que fazer com um filho que vai nascer.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Plan {
    /// nao roda: devolve saida com erro e esta mensagem no stderr
    pub fail: Option<String>,
    pub hang: bool,
    pub delay: Option<Duration>,
    /// mata depois desse tempo; `true` = a arvore inteira
    pub kill: Option<(Duration, bool)>,
    /// nome curto do filho (para o log)
    pub who: String,
}

impl Plan {
    pub fn is_noop(&self) -> bool {
        self.fail.is_none() && !self.hang && self.delay.is_none() && self.kill.is_none()
    }
}

pub const BLOCK_MSG: &str =
    "ERROR: [youtube] caos: Sign in to confirm you're not a bot. HTTP Error 429: Too Many Requests";

fn parse_dur(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num, mul) = if let Some(n) = s.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60.0)
    } else {
        (s, 1.0)
    };
    let v: f64 = num.trim().parse().ok()?;
    (v >= 0.0).then(|| Duration::from_secs_f64(v * mul))
}

fn parse_kind(s: &str) -> Option<Kind> {
    Some(match s {
        "kill" => Kind::Kill,
        "crash" => Kind::Crash,
        "fail" => Kind::Fail,
        "block" => Kind::Block,
        "delay" => Kind::Delay,
        "hang" => Kind::Hang,
        "full" => Kind::Full,
        "empty" => Kind::Empty,
        "truncate" => Kind::Truncate,
        "ratelimit" => Kind::RateLimit,
        _ => return None,
    })
}

/// Le as regras; regra invalida e erro (melhor nao rodar do que rodar um experimento diferente do escrito).
pub fn parse(spec: &str) -> Result<Vec<Rule>, String> {
    let mut out = Vec::new();
    for raw in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = raw.split(':').collect();
        let bad = || format!("regra de caos invalida: \"{raw}\"");
        if parts.len() < 2 {
            return Err(bad());
        }
        let kind = parse_kind(parts[0]).ok_or_else(bad)?;
        let target = parts[1].trim().to_lowercase();
        if target.is_empty() {
            return Err(bad());
        }
        let arg = parts.get(2).copied();
        let (prob, dur) = match kind {
            Kind::Delay => (1.0, parse_dur(arg.ok_or_else(bad)?).ok_or_else(bad)?),
            Kind::Kill | Kind::Crash => {
                let p = arg.map(|a| a.parse::<f64>().map_err(|_| bad())).transpose()?.unwrap_or(1.0);
                let d = parts.get(3).map(|a| parse_dur(a).ok_or_else(bad)).transpose()?.unwrap_or(Duration::from_secs(10));
                (p, d)
            }
            _ => (arg.map(|a| a.parse::<f64>().map_err(|_| bad())).transpose()?.unwrap_or(1.0), Duration::ZERO),
        };
        if !(0.0..=1.0).contains(&prob) {
            return Err(bad());
        }
        out.push(Rule { kind, target, prob, dur });
    }
    Ok(out)
}

impl Monkey {
    pub fn new(rules: Vec<Rule>, seed: u64) -> Monkey {
        Monkey { rules, seed, rng: Mutex::new(StdRng::seed_from_u64(seed)) }
    }

    fn roll(&self, p: f64) -> bool {
        if p >= 1.0 {
            return true;
        }
        let mut r = self.rng.lock().unwrap_or_else(|e| e.into_inner());
        r.gen::<f64>() < p
    }

    fn pick_dur(&self, max: Duration) -> Duration {
        let mut r = self.rng.lock().unwrap_or_else(|e| e.into_inner());
        // nunca no instante zero: a ideia e matar NO MEIO, nao antes de nascer
        let lo = (max.as_secs_f64() * 0.1).min(0.5);
        Duration::from_secs_f64(r.gen_range(lo..=max.as_secs_f64().max(lo)))
    }

    fn matching<'a>(&'a self, names: &'a [String]) -> impl Iterator<Item = &'a Rule> + 'a {
        self.rules.iter().filter(move |r| r.target == "*" || names.iter().any(|n| *n == r.target))
    }

    /// Plano para um filho com esses nomes (executavel + scripts).
    pub fn plan(&self, names: &[String]) -> Plan {
        let mut p = Plan { who: names.join("/"), ..Plan::default() };
        for r in self.matching(names) {
            match r.kind {
                Kind::Fail if p.fail.is_none() && self.roll(r.prob) => {
                    p.fail = Some(format!("caos: falha injetada em {}", p.who));
                }
                Kind::Block if p.fail.is_none() && self.roll(r.prob) => p.fail = Some(BLOCK_MSG.into()),
                Kind::Hang if !p.hang && self.roll(r.prob) => p.hang = true,
                Kind::Delay if p.delay.is_none() => p.delay = Some(r.dur),
                Kind::Kill | Kind::Crash if p.kill.is_none() && self.roll(r.prob) => {
                    p.kill = Some((self.pick_dur(r.dur), r.kind == Kind::Kill));
                }
                _ => {}
            }
        }
        p
    }

    /// Gancho fora de processo (`write`, `claude`...): sorteia se esse tipo de falha acontece agora.
    pub fn hit(&self, kind: Kind, target: &str) -> bool {
        self.rules.iter().any(|r| r.kind == kind && (r.target == target || r.target == "*") && self.roll(r.prob))
    }
}

/// O macaco ligado (ou `None`). Le o ambiente uma vez so.
pub fn monkey() -> Option<&'static Monkey> {
    static M: OnceLock<Option<Monkey>> = OnceLock::new();
    M.get_or_init(|| {
        let spec = std::env::var("TKD_CHAOS").ok().filter(|s| !s.trim().is_empty())?;
        let rules = match parse(&spec) {
            Ok(r) => r,
            Err(e) => {
                // regra errada nao pode virar "caos desligado sem aviso": aborta
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
        let seed = std::env::var("TKD_CHAOS_SEED").ok().and_then(|s| s.trim().parse().ok()).unwrap_or_else(rand::random);
        tracing::warn!("CAOS LIGADO: {spec} (semente {seed}; repita com TKD_CHAOS_SEED={seed})");
        Some(Monkey::new(rules, seed))
    })
    .as_ref()
}

pub fn enabled() -> bool {
    monkey().is_some()
}

/// Teto de tempo encurtado nos experimentos: com o macaco ligado e `TKD_CHAOS_LIMIT` (ex.: `20s`), todo teto
/// (navegador, `claude -p`, filhos com `tree_output_timeout`) vira no maximo esse valor. Assim um `hang`
/// prova o timeout em segundos, nao em 30 min. Fora do caos nao muda nada.
pub fn limit(d: Duration) -> Duration {
    if !enabled() {
        return d;
    }
    match std::env::var("TKD_CHAOS_LIMIT").ok().and_then(|s| parse_dur(&s)) {
        Some(cap) if cap < d => cap,
        _ => d,
    }
}

/// Nomes pelos quais um filho pode ser alvo: executavel e scripts passados como argumento.
pub fn names_of(program: &std::ffi::OsStr, args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> Vec<String> {
    let stem = |s: &std::ffi::OsStr| {
        std::path::Path::new(s).file_stem().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default()
    };
    let mut v = vec![stem(program)];
    for a in args {
        let a = a.as_ref();
        let p = std::path::Path::new(a);
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if matches!(ext.as_str(), "py" | "mjs" | "js" | "cjs" | "ps1") {
            v.push(stem(a));
        } else if v.len() == 1 && !a.to_string_lossy().starts_with('-') && v[0] == "npx" {
            // npx remotion render ...
            v.push(a.to_string_lossy().to_lowercase());
        }
    }
    v.retain(|s| !s.is_empty());
    v.dedup();
    v
}

/// Plano para um comando (vazio se o macaco estiver desligado).
pub fn plan_for(cmd: &tokio::process::Command) -> Plan {
    let Some(m) = monkey() else { return Plan::default() };
    let std = cmd.as_std();
    let plan = m.plan(&names_of(std.get_program(), std.get_args()));
    if !plan.is_noop() {
        note("process", &plan.who, &format!("{plan:?}"));
    }
    plan
}

/// Gancho fora de processo; registra quando acerta.
pub fn hit(kind: Kind, target: &str) -> bool {
    let Some(m) = monkey() else { return false };
    let h = m.hit(kind, target);
    if h {
        note("point", target, &format!("{kind:?}"));
    }
    h
}

/// Registra a injecao no log e em `out/chaos/injections.jsonl` (o relatorio do `tkd chaos` le daqui).
pub fn note(kind: &str, who: &str, what: &str) {
    tracing::warn!("caos [{kind}] {who}: {what}");
    let dir = crate::config::root().join("out").join("chaos");
    let _ = std::fs::create_dir_all(&dir);
    let line = serde_json::json!({ "at": crate::state::now(), "pid": std::process::id(), "kind": kind, "who": who, "what": what });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("injections.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
}

/// Saida falsa de um filho que "saiu com erro".
pub fn failed_status() -> std::process::ExitStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1 << 8)
    }
}

pub fn failed_output(msg: &str) -> std::process::Output {
    std::process::Output { status: failed_status(), stdout: Vec::new(), stderr: msg.as_bytes().to_vec() }
}

/// Mata o processo `pid` (so ele, ou a arvore) depois de `after`, a nao ser que o guard caia antes.
pub struct Killer(Option<tokio::task::JoinHandle<()>>);

impl Drop for Killer {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

impl Killer {
    pub fn none() -> Killer {
        Killer(None)
    }

    /// Arma a morte de `pid` segundo o plano.
    pub fn arm(plan: &Plan, pid: Option<u32>) -> Killer {
        let (Some((after, tree)), Some(pid)) = (plan.kill, pid) else { return Killer(None) };
        let who = plan.who.clone();
        Killer(Some(tokio::spawn(async move {
            tokio::time::sleep(after).await;
            note("kill", &who, &format!("pid {pid} apos {:.1}s{}", after.as_secs_f64(), if tree { " (arvore)" } else { "" }));
            kill_pid(pid, tree).await;
        })))
    }
}

async fn kill_pid(pid: u32, tree: bool) {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("taskkill");
        c.args(["/F", "/PID", &pid.to_string()]);
        if tree {
            c.arg("/T");
        }
        c.creation_flags(0x0800_0000);
        let _ = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
    }
    #[cfg(not(windows))]
    {
        let _ = tree;
        let _ = tokio::process::Command::new("kill").args(["-9", &pid.to_string()]).status().await;
    }
}

/// Aplica atraso/travamento antes de esperar o filho.
pub async fn stall(plan: &Plan) {
    if let Some(d) = plan.delay {
        tokio::time::sleep(d).await;
    }
    if plan.hang {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rules() {
        let r = parse("kill:ffmpeg:0.2, delay:claude:30s,fail:yt-dlp:1.0,hang:npx").unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0], Rule { kind: Kind::Kill, target: "ffmpeg".into(), prob: 0.2, dur: Duration::from_secs(10) });
        assert_eq!(r[1].dur, Duration::from_secs(30));
        assert_eq!(r[2].kind, Kind::Fail);
        assert_eq!((r[3].kind, r[3].prob), (Kind::Hang, 1.0));
        assert_eq!(parse("kill:npx:1:500ms").unwrap()[0].dur, Duration::from_millis(500));
        assert_eq!(parse("delay:node:2m").unwrap()[0].dur, Duration::from_secs(120));
    }

    #[test]
    fn bad_rule_is_error() {
        for s in ["kill", "boom:ffmpeg", "kill:ffmpeg:2", "delay:ffmpeg", "fail::1", "kill:x:0.5:abc"] {
            assert!(parse(s).is_err(), "{s}");
        }
    }

    #[test]
    fn same_seed_same_draw() {
        let rules = parse("kill:ffmpeg:0.5").unwrap();
        let names = vec!["ffmpeg".to_string()];
        let a: Vec<_> = { let m = Monkey::new(rules.clone(), 7); (0..20).map(|_| m.plan(&names)).collect() };
        let b: Vec<_> = { let m = Monkey::new(rules, 7); (0..20).map(|_| m.plan(&names)).collect() };
        assert_eq!(a, b);
        assert!(a.iter().any(|p| p.kill.is_some()) && a.iter().any(|p| p.kill.is_none()));
    }

    #[test]
    fn targets_by_exe_or_script() {
        let n = names_of("C:\\Python\\python.exe".as_ref(), ["-I", "scripts\\speech.py", "x.mp4"]);
        assert_eq!(n, vec!["python", "speech"]);
        let n = names_of("npx.cmd".as_ref(), ["remotion", "render", "Comp"]);
        assert_eq!(n, vec!["npx", "remotion"]);
        let m = Monkey::new(parse("fail:speech:1,hang:ffmpeg").unwrap(), 1);
        assert!(m.plan(&n).is_noop());
        let p = m.plan(&["python".into(), "speech".into()]);
        assert!(p.fail.is_some() && !p.hang);
        assert!(Monkey::new(parse("block:*:1").unwrap(), 1).plan(&["yt-dlp".into()]).fail.unwrap().contains("not a bot"));
    }

    #[test]
    fn death_midway() {
        let m = Monkey::new(parse("kill:ffmpeg:1:4s").unwrap(), 3);
        for _ in 0..50 {
            let (d, tree) = m.plan(&["ffmpeg".into()]).kill.unwrap();
            assert!(tree && d >= Duration::from_millis(400) && d <= Duration::from_secs(4));
        }
    }

    #[test]
    fn dot_hooks() {
        let m = Monkey::new(parse("full:write:1,empty:claude:0").unwrap(), 1);
        assert!(m.hit(Kind::Full, "write"));
        assert!(!m.hit(Kind::Fail, "write"));
        assert!(!m.hit(Kind::Empty, "claude"));
    }
}
