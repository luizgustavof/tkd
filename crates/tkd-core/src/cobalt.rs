//! Cobalt (github.com/imputnet/cobalt): baixa o video pelo servidor de uma instancia, com o IP DELA.
//! Fica na frente do yt-dlp no download de footage: se nenhuma instancia entrega, o yt-dlp assume.
//!
//! Instancias em `tkd.toml`:
//!   [cobalt]
//!   instances = ["https://rue-cobalt.xenon.zone/"]
//! Chaves de API (quando o dono de uma instancia libera) ficam fora do git, em `cobalt.local.toml`:
//!   [keys]
//!   "https://instancia.exemplo/" = "chave"
//!
//! Instancia com captcha (Turnstile, erro `auth.jwt.missing`) e pulada: o dono quis barrar uso
//! automatico, e nao burlamos isso. Sem chave, so servem as abertas.

use crate::proc::TreeExt;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Instancia que falhou fica de lado por um tempo (por processo).
static BENCHED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
const FORA_FALHA: Duration = Duration::from_secs(60 * 60);
const FORA_CAPTCHA: Duration = Duration::from_secs(24 * 60 * 60);
/// Abaixo disso o "video" e lixo (tunel vazio quando o YouTube barra a instancia).
const MIN_BYTES: u64 = 200_000;

#[derive(Debug, Clone)]
pub struct Instance {
    pub url: String,
    pub key: Option<String>,
}

fn norm(u: &str) -> String {
    let u = u.trim();
    if u.ends_with('/') { u.to_string() } else { format!("{u}/") }
}

/// Instancias configuradas (ordem do tkd.toml), com a chave se houver.
pub fn instances() -> Vec<Instance> {
    let root = crate::config::root();
    let read_toml = |p: &Path| std::fs::read_to_string(p).ok().and_then(|t| toml::from_str::<toml::Value>(&t).ok());
    let keys: HashMap<String, String> = read_toml(&root.join("cobalt.local.toml"))
        .and_then(|v| v.get("keys").and_then(|k| k.as_table()).cloned())
        .map(|t| t.into_iter().filter_map(|(k, v)| Some((norm(&k), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    read_toml(&root.join("tkd.toml"))
        .and_then(|v| v.get("cobalt")?.get("instances")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|u| u.starts_with("https://"))
        .map(|u| {
            let url = norm(u);
            Instance { key: keys.get(&url).cloned(), url }
        })
        .collect()
}

fn is_benched(url: &str) -> bool {
    BENCHED.lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(url).copied()))
        .is_some_and(|up_to| Instant::now() < up_to)
}

fn bench_instance(url: &str, per_unit: Duration) {
    if let Ok(mut g) = BENCHED.lock() {
        g.get_or_insert_with(HashMap::new).insert(url.to_string(), Instant::now() + per_unit);
    }
}

/// Erro do cobalt que significa "esta instancia exige captcha/login dela".
pub fn is_captcha(code: &str) -> bool {
    code.contains("auth.jwt") || code.contains("auth.turnstile")
}

/// Pede o video a uma instancia: devolve o link do arquivo (tunnel/redirect).
async fn request_one(inst: &Instance, video: &str) -> Result<String> {
    let body = serde_json::json!({
        "url": video,
        "videoQuality": "1080",
        "youtubeVideoCodec": "h264",
        "downloadMode": "auto",
        "filenameStyle": "basic",
    })
    .to_string();
    let mut cmd = tokio::process::Command::new("curl");
    cmd.args(["-s", "-m", "60", "-X", "POST", &inst.url])
        .args(["-H", "Accept: application/json", "-H", "Content-Type: application/json"])
        .args(["-A", "tkd/1.0"]);
    if let Some(k) = &inst.key {
        cmd.arg("-H").arg(format!("Authorization: Api-Key {k}"));
    }
    let out = cmd.arg("--data-binary").arg(&body).tree_output().await.context("executando curl")?;
    let v: Value = serde_json::from_slice(&out.stdout)
        .with_context(|| format!("resposta invalida: {}", String::from_utf8_lossy(&out.stdout).chars().take(120).collect::<String>()))?;
    match v["status"].as_str().unwrap_or("") {
        "tunnel" | "redirect" => v["url"].as_str().map(String::from).context("sem url na resposta"),
        "error" => {
            let code = v["error"]["code"].as_str().unwrap_or("?").to_string();
            anyhow::bail!("{code}")
        }
        s => anyhow::bail!("status \"{s}\" nao suportado"),
    }
}

/// Baixa `video` (link do YouTube etc.) para `dst` pela primeira instancia que entregar
/// um arquivo de verdade em 720p+. Erro = nenhuma serviu (o chamador cai no yt-dlp).
pub async fn download_video(video: &str, dst: &Path) -> Result<String> {
    download_with(&instances(), video, dst).await
}

async fn download_with(inst_list: &[Instance], video: &str, dst: &Path) -> Result<String> {
    anyhow::ensure!(!inst_list.is_empty(), "nenhuma instancia do cobalt configurada");
    let mut error_list = Vec::new();
    for inst in inst_list.iter().filter(|i| !is_benched(&i.url)) {
        let r = async {
            let link = request_one(inst, video).await?;
            let _ = std::fs::remove_file(dst);
            let st = tokio::process::Command::new("curl")
                .args(["-s", "-L", "-m", "900", "-A", "tkd/1.0", "-o"])
                .arg(dst)
                .arg(&link)
                .tree_status()
                .await
                .context("executando curl")?;
            let n = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
            anyhow::ensure!(st.success() && n >= MIN_BYTES, "arquivo vazio ({n} bytes): a instancia tambem esta barrada");
            let h = crate::media::probe(dst).await.map(|p| p.height).unwrap_or(0);
            anyhow::ensure!(h >= 720, "entregou {h}p (minimo 720p)");
            Ok::<_, anyhow::Error>(h)
        }
        .await;
        match r {
            Ok(h) => {
                tracing::info!("cobalt: baixado por {} ({h}p)", inst.url);
                return Ok(inst.url.clone());
            }
            Err(e) => {
                let msg = format!("{e:#}");
                let _ = std::fs::remove_file(dst);
                bench_instance(&inst.url, if is_captcha(&msg) { FORA_CAPTCHA } else { FORA_FALHA });
                tracing::info!("cobalt: {} falhou: {msg}", inst.url);
                error_list.push(format!("{}: {msg}", inst.url));
            }
        }
    }
    anyhow::bail!("cobalt sem instancia util ({})", if error_list.is_empty() { "todas em espera".into() } else { error_list.join("; ") })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Instancia falsa em 127.0.0.1 que responde sempre `body` (HTTP 200) e devolve a URL dela.
    fn fake(body: &'static str) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        url
    }

    /// C3 do CHAOS.md: todas as instancias fora (porta fechada, erro, captcha, tunel vazio) = erro rapido que lista
    /// cada uma, sem arquivo largado (o footage cai para o yt-dlp com esse erro no log).
    #[tokio::test]
    async fn all_out_fails_fast_and_clean() {
        let closed = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/", l.local_addr().unwrap())
        };
        let tiny = fake(r#"{"status":"tunnel","url":"http://127.0.0.1:9/nada"}"#);
        let inst_list: Vec<Instance> = [
            closed,
            fake(r#"{"status":"error","error":{"code":"error.api.youtube.login"}}"#),
            fake(r#"{"status":"error","error":{"code":"error.api.auth.jwt.missing"}}"#),
            fake("<html>fora do ar</html>"),
            tiny,
        ]
        .into_iter()
        .map(|url| Instance { url, key: None })
        .collect();
        let dst = std::env::temp_dir().join(format!("tkd-test-cobalt-{}.mp4", std::process::id()));
        let t = Instant::now();
        let e = format!("{:#}", download_with(&inst_list, "https://www.youtube.com/watch?v=x", &dst).await.unwrap_err());
        assert!(t.elapsed() < Duration::from_secs(30), "demorou {:?}", t.elapsed());
        assert!(e.contains("youtube.login") && e.contains("auth.jwt") && e.contains("resposta invalida") && e.contains("arquivo vazio"), "{e}");
        assert!(!dst.exists());
        // todas ficam de lado: a proxima tentativa nem pede (sem martelar instancia fora)
        let e2 = format!("{:#}", download_with(&inst_list, "https://www.youtube.com/watch?v=x", &dst).await.unwrap_err());
        assert!(e2.contains("todas em espera"), "{e2}");
    }

    #[test]
    fn normalizes_and_captcha() {
        assert_eq!(norm("https://a.b"), "https://a.b/");
        assert_eq!(norm("https://a.b/"), "https://a.b/");
        assert!(is_captcha("error.api.auth.jwt.missing"));
        assert!(!is_captcha("error.api.youtube.login"));
    }

    /// rede: `cargo test -p tkd-core cobalt_ao_vivo -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn cobalt_alive() {
        let dst = std::env::temp_dir().join("tkd-cobalt-teste.mp4");
        let r = download_video("https://www.youtube.com/watch?v=k18gSPjWDM0", &dst).await;
        println!("instancias: {:?}", instances().iter().map(|i| &i.url).collect::<Vec<_>>());
        println!("resultado: {r:?}");
        let _ = std::fs::remove_file(dst);
    }

    #[test]
    fn instance_is_benched() {
        bench_instance("https://x.test/", Duration::from_secs(60));
        assert!(is_benched("https://x.test/"));
        assert!(!is_benched("https://y.test/"));
    }
}
