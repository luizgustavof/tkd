//! Processos filhos que morrem junto com quem os pediu.
//!
//! `kill_on_drop` do tokio so mata o filho direto: `npx.cmd` (cmd.exe) morre e o node/Chromium do
//! Remotion segue rodando; `python eval_video.py` morre e o llama-server fica com a VRAM. Aqui cada
//! filho vai para um Job Object proprio com KILL_ON_JOB_CLOSE: quando o guard cai (fim normal,
//! erro, `abort()` da task ou timeout), a arvore inteira cai junto.

use std::io;
use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;
use tokio::process::{Child, Command};

/// Guard da arvore de processos de um filho. Dropar = matar tudo que ainda estiver vivo nela.
pub struct Tree {
    #[cfg(windows)]
    #[allow(dead_code)] // so existe para o Drop fechar o job
    job: Option<JobHandle>,
}

impl Tree {
    /// Poe o filho (e o que ele abrir depois) num job proprio.
    pub fn adopt(child: &Child) -> Tree {
        #[cfg(windows)]
        {
            let job = child.raw_handle().and_then(|h| JobHandle::new().filter(|j| j.assign(h)));
            if job.is_none() {
                tracing::debug!("job object do filho nao aplicado: netos podem sobrar se ele for cancelado");
            }
            Tree { job }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Tree {}
        }
    }

    /// Mata agora tudo que ainda estiver vivo na arvore (sem esperar o drop).
    pub fn terminate(&self) {
        #[cfg(windows)]
        if let Some(j) = &self.job {
            // SAFETY: handle de job valido enquanto o Tree existir.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(j.0, 1);
            }
        }
    }
}

/// CREATE_NO_WINDOW: nada de console piscando no app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// BELOW_NORMAL_PRIORITY_CLASS: o Windows passa a frente o que o usuario esta usando. Os netos herdam
/// (filho abaixo do normal cria neto abaixo do normal), entao o ffmpeg aberto pelo python tambem cede.
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Prioridade dos filhos pesados (ffmpeg, Remotion, whisper, llama-server, yt-dlp, vigias de live).
/// `TKD_CHILD_PRIORITY=normal` volta ao normal. Nunca IDLE: a captura de live perderia segmento com o PC ocupado.
pub fn background_flags() -> u32 {
    #[cfg(windows)]
    {
        let normal = std::env::var("TKD_CHILD_PRIORITY").is_ok_and(|v| v.eq_ignore_ascii_case("normal"));
        if normal { CREATE_NO_WINDOW } else { CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Aplica `background_flags` (sem janela + abaixo do normal) num comando.
pub fn background(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(background_flags());
    cmd
}

/// Folga para os netos soltarem stdout/stderr depois que o filho direto saiu.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Igual a `Child::wait_with_output`, mas nao fica preso a um neto: se o filho direto saiu (ou morreu)
/// e um neto segue segurando o stdout, espera `PIPE_GRACE` e derruba a arvore. Antes, matar so o
/// `npx.cmd` deixava node/Chromium renderizando ate o fim com o tkd esperando o pipe fechar.
pub async fn wait_output(mut child: Child, tree: &Tree) -> io::Result<Output> {
    use tokio::io::AsyncReadExt;
    async fn drain(p: Option<impl tokio::io::AsyncRead + Unpin>) -> Vec<u8> {
        let mut v = Vec::new();
        if let Some(mut p) = p {
            let _ = p.read_to_end(&mut v).await;
        }
        v
    }
    let pipes = futures_join(drain(child.stdout.take()), drain(child.stderr.take()));
    tokio::pin!(pipes);
    let mut early = None;
    // le os pipes enquanto espera o filho (pipe cheio travaria o filho)
    let status = tokio::select! {
        st = child.wait() => st?,
        r = &mut pipes => { early = Some(r); child.wait().await? }
    };
    let (stdout, stderr) = match early {
        Some(r) => r,
        None => match tokio::time::timeout(PIPE_GRACE, &mut pipes).await {
            Ok(r) => r,
            Err(_) => {
                tracing::debug!("filho saiu ({status}) mas um neto segurava o pipe: derrubando a arvore");
                tree.terminate();
                pipes.await
            }
        },
    };
    Ok(Output { status, stdout, stderr })
}

async fn futures_join<A: std::future::Future, B: std::future::Future>(a: A, b: B) -> (A::Output, B::Output) {
    tokio::join!(a, b)
}

/// Saida e status com a arvore presa ao future: cancelar o future mata todos os processos.
pub trait TreeExt {
    /// Igual a `Command::output`, mas mata a arvore no drop e aceita teto de tempo.
    fn tree_output(&mut self) -> impl std::future::Future<Output = io::Result<Output>> + Send + use<Self>;
    /// `tree_output` com teto: estourou = mata a arvore e devolve `TimedOut`.
    fn tree_output_timeout(&mut self, limit: Duration) -> impl std::future::Future<Output = io::Result<Output>> + Send + use<Self>;
    /// Igual a `Command::status`, mas mata a arvore no drop.
    fn tree_status(&mut self) -> impl std::future::Future<Output = io::Result<ExitStatus>> + Send + use<Self>;
}

impl TreeExt for Command {
    fn tree_output(&mut self) -> impl std::future::Future<Output = io::Result<Output>> + Send + use<> {
        self.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        background(self);
        let plan = crate::chaos::plan_for(self);
        let spawned = (plan.fail.is_none() && !plan.hang).then(|| self.spawn());
        async move {
            crate::chaos::stall(&plan).await;
            let Some(spawned) = spawned else {
                return Ok(crate::chaos::failed_output(plan.fail.as_deref().unwrap_or_default()));
            };
            let child = spawned?;
            let tree = Tree::adopt(&child);
            let _chaos = crate::chaos::Killer::arm(&plan, child.id());
            wait_output(child, &tree).await
        }
    }

    fn tree_output_timeout(&mut self, limit: Duration) -> impl std::future::Future<Output = io::Result<Output>> + Send + use<> {
        let limit = crate::chaos::limit(limit);
        let fut = self.tree_output();
        async move {
            tokio::time::timeout(limit, fut)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, format!("passou de {}s", limit.as_secs())))?
        }
    }

    fn tree_status(&mut self) -> impl std::future::Future<Output = io::Result<ExitStatus>> + Send + use<> {
        self.kill_on_drop(true);
        background(self);
        let plan = crate::chaos::plan_for(self);
        let spawned = (plan.fail.is_none() && !plan.hang).then(|| self.spawn());
        async move {
            crate::chaos::stall(&plan).await;
            let Some(spawned) = spawned else {
                if let Some(msg) = &plan.fail {
                    tracing::warn!("{msg}");
                }
                return Ok(crate::chaos::failed_status());
            };
            let mut child = spawned?;
            let _tree = Tree::adopt(&child);
            let _chaos = crate::chaos::Killer::arm(&plan, child.id());
            child.wait().await
        }
    }
}

/// Caos para quem faz o spawn na mao (render, agente, navegador): aplica falha/atraso/travamento e
/// devolve o plano para armar a morte com `chaos::Killer::arm` depois do spawn.
pub async fn chaos_gate(cmd: &Command) -> io::Result<crate::chaos::Plan> {
    let plan = crate::chaos::plan_for(cmd);
    crate::chaos::stall(&plan).await;
    match &plan.fail {
        Some(msg) => Err(io::Error::other(msg.clone())),
        None => Ok(plan),
    }
}

/// Trava entre processos (app + CLI): arquivo aberto em modo exclusivo. Cai sozinha quando o guard sai
/// de escopo ou o processo morre (o Windows fecha o handle), entao nao ha trava esquecida.
pub struct ExclusiveFile(#[allow(dead_code)] std::fs::File);

impl ExclusiveFile {
    /// Tenta pegar agora; `None` = outro processo esta com ela.
    pub fn try_acquire(path: &std::path::Path) -> Option<ExclusiveFile> {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.share_mode(0);
        }
        let f = o.open(path).ok()?;
        Some(ExclusiveFile(f))
    }

    /// Espera a trava (consultando a cada segundo).
    pub async fn acquire(path: &std::path::Path, on_wait: impl FnOnce()) -> ExclusiveFile {
        if let Some(l) = Self::try_acquire(path) {
            return l;
        }
        on_wait();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Some(l) = Self::try_acquire(path) {
                return l;
            }
        }
    }
}

/// O processo `pid` ainda esta rodando?
pub fn pid_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        // SAFETY: handle aberto aqui e fechado aqui; ponteiro para u32 local.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Apaga o arquivo ou a pasta quando sai de escopo (inclusive por `?` ou cancelamento).
pub struct RemoveOnDrop(pub std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.0.is_dir() {
            // pasta de trabalho de processo filho recem-morto (Chromium do Remotion): o Windows ainda segura os
            // arquivos por um instante e o remove_dir_all falhava pela metade, largando a pasta
            for _ in 0..10 {
                if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// Destino de download e os pedacos que o yt-dlp/curl deixam ao lado dele (`x.mp4.part`, `x.f137.mp4`,
/// `x.f251.webm.part`, `x.mp4.ytdl`, `x.temp.mp4`). Limpa ao criar (sobra de um processo morto, que nao rodou o
/// Drop: o yt-dlp RETOMARIA o `.part` de outro video com o mesmo nome) e ao sair.
pub struct RemoveFamily(pub std::path::PathBuf);

impl RemoveFamily {
    pub fn new(path: std::path::PathBuf) -> Self {
        let g = Self(path);
        g.clean();
        g
    }

    fn clean(&self) {
        let Some(dir) = self.0.parent() else { return };
        let Some(name) = self.0.file_name().map(|n| n.to_string_lossy().to_string()) else { return };
        let stem = self.0.file_stem().map(|n| format!("{}.", n.to_string_lossy())).unwrap_or_else(|| name.clone());
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if (n == name || n.starts_with(&stem)) && e.path().is_file() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Drop for RemoveFamily {
    fn drop(&mut self) {
        self.clean();
    }
}

/// Poe o processo atual num Job Object com KILL_ON_JOB_CLOSE: quando o app/CLI fecha (ou cai),
/// os filhos (`claude -p`, node/Edge, Remotion, ffmpeg, live.py) morrem junto em vez de seguirem
/// rodando orfaos e gastando tokens.
pub fn adopt_current_process() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let Some(job) = JobHandle::new() else {
            tracing::warn!("sem job object: filhos podem sobrar se o processo cair");
            return;
        };
        // SAFETY: GetCurrentProcess devolve um pseudo-handle sempre valido.
        let ok = unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } != 0;
        if !ok {
            tracing::warn!("job object nao aplicado: filhos podem sobrar se o processo cair");
            return;
        }
        // o handle fica aberto ate o processo morrer: e o fechamento dele que mata os filhos
        std::mem::forget(job);
    }
}

#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: um handle de job e so um numero do kernel; usar de outra thread e permitido.
#[cfg(windows)]
unsafe impl Send for JobHandle {}
#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn new() -> Option<JobHandle> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: chamadas Win32 com ponteiros validos para structs locais.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let handle = JobHandle(job);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0;
            ok.then_some(handle)
        }
    }

    fn assign(&self, process: std::os::windows::io::RawHandle) -> bool {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: `process` vem de um Child vivo (tokio guarda o handle aberto).
        unsafe { AssignProcessToJobObject(self.0, process as _) != 0 }
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        // SAFETY: handle criado por CreateJobObjectW e fechado uma vez so.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tree_output_returns_stdout() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo ok"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "echo ok"]);
            c
        };
        let out = cmd.tree_output().await.unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
    }

    #[tokio::test]
    async fn tree_output_timeout_fails_fast() {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-Command", "Start-Sleep 30"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let t0 = std::time::Instant::now();
        let err = cmd.tree_output_timeout(Duration::from_millis(500)).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(t0.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn download_family() {
        let d = std::env::temp_dir().join(format!("tkd-test-family-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for n in ["v.mp4", "v.mp4.part", "v.f137.mp4", "v.f251.webm.part", "v.mp4.ytdl", "v2.mp4", "vx.mp4"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        {
            let _g = RemoveFamily::new(d.join("v.mp4"));
            let left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            assert_eq!(left.len(), 2, "{left:?}");
            std::fs::write(d.join("v.mp4.part"), b"x").unwrap();
        }
        assert!(!d.join("v.mp4.part").exists() && d.join("v2.mp4").exists() && d.join("vx.mp4").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn exclusive_lock_across_handles() {
        let p = std::env::temp_dir().join(format!("tkd-lock-{}.lock", std::process::id()));
        let a = ExclusiveFile::try_acquire(&p).expect("primeira pega");
        if cfg!(windows) {
            assert!(ExclusiveFile::try_acquire(&p).is_none(), "segunda nao pode pegar");
        }
        drop(a);
        assert!(ExclusiveFile::try_acquire(&p).is_some(), "liberou no drop");
        let _ = std::fs::remove_file(&p);
    }

    /// Filho e neto (cmd -> powershell) rodam abaixo do normal: o PC segue usavel com os canais rodando.
    #[cfg(windows)]
    #[tokio::test]
    async fn children_run_below_normal() {
        if std::env::var("TKD_CHILD_PRIORITY").is_ok() {
            return;
        }
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "powershell -NoProfile -Command (Get-Process -Id $PID).PriorityClass"]);
        let out = cmd.tree_output().await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "BelowNormal");
    }

    /// Filho direto morto com o neto segurando o stdout: nao espera o neto terminar.
    #[cfg(windows)]
    #[tokio::test]
    async fn dead_child_does_not_wait_for_grandchild() {
        // cmd /C start /B ping (neto herda o stdout) e o cmd sai na hora
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start /B ping -n 30 127.0.0.1 >NUL & echo pai"]);
        let t0 = std::time::Instant::now();
        let out = cmd.tree_output().await.unwrap();
        assert!(t0.elapsed() < Duration::from_secs(10), "esperou o neto: {:?}", t0.elapsed());
        assert!(String::from_utf8_lossy(&out.stdout).contains("pai"));
    }

    /// cmd.exe -> ping: estourar o tempo tem que matar o neto, nao so o cmd.
    #[cfg(windows)]
    #[tokio::test]
    async fn timeout_kills_grandchildren() {
        // -w com valor unico identifica o neto na lista de processos
        let marker = 4000 + std::process::id() % 1000;
        let count = || async move {
            let q = format!(
                "@(Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | Where-Object {{ $_.CommandLine -like '*-w {marker} *' }}).Count"
            );
            let out = Command::new("powershell").args(["-NoProfile", "-Command", &q]).output().await.unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", &format!("ping -n 60 -w {marker} 127.0.0.1")]);
        let run = cmd.tree_output_timeout(Duration::from_secs(5));
        let (r, alive) = tokio::join!(run, async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            count().await
        });
        assert!(r.is_err());
        assert_eq!(alive, "1", "neto nem chegou a rodar");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(count().await, "0", "neto sobreviveu");
    }
}
