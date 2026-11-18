//! Conferencia da primeira execucao: o que falta instalar para os canais rodarem, com o conserto de cada item
//! (comando para copiar e, quando da para fazer sem risco, um conserto de um clique).

use crate::config::root;
use crate::proc::TreeExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    /// funciona, mas algo pode dar errado (versao velha, login nao confirmado)
    Warn,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    pub status: Status,
    /// o que foi encontrado (versao, caminho) ou o que falta
    pub detail: String,
    /// para que serve, em uma frase
    pub why: &'static str,
    /// comando para o usuario rodar (copiar e colar), se houver
    pub command: Option<String>,
    /// conserto que o app faz sozinho (`fix`), se houver
    pub fix: Option<&'static str>,
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// Roda `bin args` e devolve a primeira linha da saida (stdout ou stderr), ou o erro.
async fn version_of(bin: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    match cmd.tree_output_timeout(TIMEOUT).await {
        Ok(o) if o.status.success() => {
            let text = if o.stdout.is_empty() { o.stderr } else { o.stdout };
            Ok(String::from_utf8_lossy(&text).lines().next().unwrap_or("").trim().to_string())
        }
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("falhou").trim().to_string()),
        Err(e) => Err(if e.kind() == std::io::ErrorKind::NotFound { "nao encontrado".into() } else { e.to_string() }),
    }
}

/// Primeiro numero de uma versao ("v20.11.1" -> 20, "ffmpeg version 7.1-full" -> 7).
pub fn major(v: &str) -> u32 {
    v.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn item(id: &'static str, label: &'static str, why: &'static str) -> Item {
    Item { id, label, status: Status::Missing, detail: String::new(), why, command: None, fix: None }
}

async fn ffmpeg() -> Item {
    let mut it = item("ffmpeg", "FFmpeg 9+", "corta, mede e monta todos os videos");
    let bin = PathBuf::from(crate::media::ffmpeg_bin());
    match version_of(&bin, &["-hide_banner", "-version"]).await {
        Ok(v) => {
            let ver = v.split_whitespace().nth(2).unwrap_or(&v).to_string();
            it.status = if major(&ver) >= 9 { Status::Ok } else { Status::Warn };
            it.detail = if it.status == Status::Ok { ver } else { format!("{ver} (o projeto espera 9+)") };
            if it.status != Status::Ok {
                it.command = Some("winget upgrade Gyan.FFmpeg".into());
            }
        }
        Err(e) => {
            it.detail = e;
            it.command = Some("winget install Gyan.FFmpeg".into());
        }
    }
    it
}

async fn node() -> Item {
    let mut it = item("node", "Node.js 20+", "abre o Edge para login e upload (Playwright)");
    match version_of(Path::new("node"), &["--version"]).await {
        Ok(v) => {
            it.status = if major(&v) >= 20 { Status::Ok } else { Status::Warn };
            it.detail = v;
        }
        Err(e) => it.detail = e,
    }
    if it.status != Status::Ok {
        it.command = Some("winget install OpenJS.NodeJS.LTS".into());
    }
    it
}

fn browser_deps() -> Item {
    let mut it = item("browser_deps", "Pacotes do navegador", "Playwright usado pelos scripts de browser/");
    let nm = root().join("browser").join("node_modules").join("playwright");
    if nm.exists() {
        it.status = Status::Ok;
        it.detail = "browser/node_modules".into();
    } else {
        it.detail = "faltam os pacotes de browser/".into();
        it.command = Some("cd browser && npm install".into());
    }
    it
}

fn edge() -> Item {
    let mut it = item("edge", "Microsoft Edge", "um perfil por canal guarda o login do TikTok/YouTube");
    let found = [r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"]
        .into_iter()
        .find(|p| Path::new(p).exists());
    match found {
        Some(p) => {
            it.status = Status::Ok;
            it.detail = p.into();
        }
        None if !cfg!(windows) => {
            it.status = Status::Warn;
            it.detail = "fora do Windows: confira o Edge a mao".into();
        }
        None => {
            it.detail = "msedge.exe nao encontrado".into();
            it.command = Some("winget install Microsoft.Edge".into());
        }
    }
    it
}

/// Login do `claude`: o arquivo de credenciais ou a chave de API. Sem chamar o modelo (custaria tokens).
fn claude_logged_in() -> bool {
    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.is_empty()) {
        return true;
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    home.is_some_and(|h| h.join(".claude").join(".credentials.json").exists())
}

async fn claude() -> Item {
    let mut it = item("claude", "Claude Code (CLI `claude`)", "todas as decisoes criativas: ideia, edit, legenda, escolha do corte");
    match version_of(Path::new("claude"), &["--version"]).await {
        Ok(v) if claude_logged_in() => {
            it.status = Status::Ok;
            it.detail = v;
        }
        Ok(v) => {
            it.status = Status::Warn;
            it.detail = format!("{v}; nao achei o login (pode estar no chaveiro do sistema)");
            it.command = Some("claude".into());
        }
        Err(e) => {
            it.detail = e;
            it.command = Some("npm install -g @anthropic-ai/claude-code".into());
        }
    }
    it
}

/// Executavel no PATH (com `.exe` no Windows).
fn on_path(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Python do yt-dlp: o que fica ao lado do `yt-dlp` no PATH (`Python312/Scripts/yt-dlp.exe` ->
/// `Python312/python.exe`). O `python` do PATH pode ser outro (uv, loja da Microsoft) e nao ver o yt-dlp-ejs.
fn ytdlp_python() -> PathBuf {
    if let Ok(p) = std::env::var("TKD_PYTHON") {
        return PathBuf::from(p);
    }
    let exe = if cfg!(windows) { "python.exe" } else { "python3" };
    if let Some(bin_dir) = on_path("yt-dlp").and_then(|p| p.parent().map(Path::to_path_buf)) {
        for cand in [bin_dir.join(exe), bin_dir.parent().map(|d| d.join(exe)).unwrap_or_default()] {
            if cand.is_file() {
                return cand;
            }
        }
    }
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

fn ytdlp_install_cmd(py: &Path) -> String {
    let py = py.display().to_string();
    let py = if py.contains(' ') { format!("\"{py}\"") } else { py };
    format!("{py} -m pip install -U \"yt-dlp[default]\"")
}

async fn ytdlp() -> Item {
    let mut it = item("ytdlp", "yt-dlp + solucionador JS", "baixa referencias, footage e VODs; sem o solucionador os formatos HD somem");
    let py = ytdlp_python();
    it.command = Some(ytdlp_install_cmd(&py));
    it.fix = Some("update_ytdlp");
    match version_of(Path::new("yt-dlp"), &["--version"]).await {
        Ok(v) => {
            let ejs = version_of(&py, &["-c", "from importlib.metadata import version; print(version('yt-dlp-ejs'))"]).await;
            match ejs {
                Ok(e) => {
                    it.status = Status::Ok;
                    it.detail = format!("{v} (solucionador {e})");
                }
                Err(_) => {
                    it.status = Status::Warn;
                    it.detail = format!("{v}, sem o yt-dlp-ejs");
                }
            }
        }
        Err(e) => it.detail = e,
    }
    it
}

async fn venv() -> Item {
    let mut it = item("venv", "Python do projeto (tools/rea-video/.venv)", "whisper, streamlink, deteccao de momentos e QA");
    let py = crate::recreate::venv_bin("python");
    if !py.exists() {
        it.detail = "venv nao criado".into();
        it.command = Some(
            "py -3.12 -m venv tools/rea-video/.venv && tools/rea-video/.venv/Scripts/pip install -r scripts/requirements.txt"
                .into(),
        );
        return it;
    }
    match version_of(&py, &["-c", "import faster_whisper, streamlink, cv2, numpy, librosa; import sys; print(sys.version.split()[0])"]).await {
        Ok(v) if !crate::recreate::venv_bin("rea-video").exists() => {
            // o formato edit (recreate, trilha da ref) chama o `rea-video decompose`; cortes de live nao
            it.status = Status::Warn;
            it.detail = format!("Python {v}, sem o rea-video (formato edit nao funciona)");
            it.command = Some("instalar.bat (baixa e instala o rea-video em tools/rea-video)".into());
        }
        Ok(v) => {
            it.status = Status::Ok;
            it.detail = format!("Python {v} + rea-video");
        }
        Err(e) => {
            it.status = Status::Warn;
            it.detail = format!("faltam pacotes ({e})");
            it.command = Some("tools/rea-video/.venv/Scripts/pip install -r scripts/requirements.txt".into());
        }
    }
    it
}

fn global_config() -> Item {
    let mut it = item("tkd_toml", "Configuracao global (tkd.toml)", "modelo do Claude por tarefa e piloto automatico");
    if root().join("tkd.toml").exists() {
        it.status = Status::Ok;
        it.detail = "tkd.toml".into();
    } else if root().join("tkd.example.toml").exists() {
        it.status = Status::Warn;
        it.detail = "usando os padroes (sem tkd.toml)".into();
        it.fix = Some("create_tkd_toml");
    } else {
        it.status = Status::Warn;
        it.detail = "usando os padroes".into();
    }
    it
}

fn channels() -> Item {
    let mut it = item("channels", "Primeiro canal", "cada canal tem nicho, horarios e a propria conta");
    let n = crate::config::load_channels().map(|v| v.len()).unwrap_or(0);
    if n > 0 {
        it.status = Status::Ok;
        it.detail = format!("{n} canal(is)");
    } else {
        // nao e coisa de instalar: aviso, nao "faltando" (o app nao diz "falta instalar: Primeiro canal")
        it.status = Status::Warn;
        it.detail = "nenhum canal ainda: crie em Canais > Novo canal".into();
    }
    it
}

fn evaluator() -> Item {
    let mut it = item("evaluator", "Avaliador local (opcional)", "modelo proprio que da nota aos edits antes do Claude");
    let cfg = crate::local_eval::config();
    // opcional: nunca "faltando"; sem ele a review usa o QA medido e o Claude
    it.status = Status::Ok;
    if !cfg.enabled() {
        it.detail = "nao configurado: review com QA + Claude (ver docs/EVALUATOR.md)".into();
    } else {
        match crate::local_eval::available(&cfg) {
            Ok(_) => it.detail = format!("{} ligado", cfg.name),
            Err(e) => {
                it.status = Status::Warn;
                it.detail = e;
            }
        }
    }
    it
}

/// Confere tudo (os comandos externos rodam em paralelo).
pub async fn check() -> Vec<Item> {
    let (a, b, c, d, e) = tokio::join!(ffmpeg(), node(), claude(), ytdlp(), venv());
    vec![a, c, d, e, b, browser_deps(), edge(), global_config(), evaluator(), channels()]
}

/// Conserto de um clique. So o que nao precisa de admin nem mexe fora do projeto/Python do usuario.
pub async fn fix(id: &str) -> anyhow::Result<String> {
    match id {
        "update_ytdlp" => {
            let mut cmd = Command::new(ytdlp_python());
            cmd.args(["-m", "pip", "install", "-U", "yt-dlp[default]"]);
            let o = cmd.tree_output_timeout(Duration::from_secs(300)).await?;
            anyhow::ensure!(o.status.success(), "pip falhou: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
            Ok("yt-dlp atualizado".into())
        }
        "create_tkd_toml" => {
            let dst = root().join("tkd.toml");
            anyhow::ensure!(!dst.exists(), "tkd.toml ja existe");
            std::fs::copy(root().join("tkd.example.toml"), &dst)?;
            Ok("tkd.toml criado a partir do exemplo".into())
        }
        _ => anyhow::bail!("conserto desconhecido: {id}"),
    }
}

/// Algo obrigatorio faltando? (o app abre a tela de primeira execucao)
pub fn needs_setup(items: &[Item]) -> bool {
    items.iter().any(|i| i.status == Status::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_of_versions() {
        assert_eq!(major("v20.11.1"), 20);
        assert_eq!(major("7.1-full_build-www.gyan.dev"), 7);
        assert_eq!(major("2026.11.10"), 2026);
        assert_eq!(major(""), 0);
    }

    #[tokio::test]
    async fn missing_binary_is_reported_not_panicked() {
        let r = version_of(Path::new("tkd-nao-existe-xyz"), &["--version"]).await;
        assert_eq!(r.unwrap_err(), "nao encontrado");
    }

    #[tokio::test]
    async fn unknown_fix_fails() {
        assert!(fix("rm -rf").await.is_err());
    }
}
