use crate::{browser, config, config::Channel};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct LoginOut {
    pub ok: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub dir: String,
    #[serde(rename = "alreadyLoggedIn", default)]
    pub already: bool,
}

#[derive(Debug, Deserialize)]
pub struct WhoamiOut {
    #[serde(default)]
    pub ok: bool,
    #[serde(rename = "loggedIn", default)]
    pub logged_in: bool,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub error: String,
}

/// Cada canal tem seu proprio diretorio de perfil do navegador: .profiles/<slug>/
pub fn profile_dir(slug: &str) -> std::path::PathBuf {
    config::root().join(".profiles").join(slug)
}

pub fn has_profile(slug: &str) -> bool {
    profile_dir(slug).exists()
}

pub const PLATFORMS: [&str; 2] = ["tiktok", "youtube"];

/// Abre o navegador no perfil do canal para login manual (TikTok ou YouTube),
/// grava a conta em `[accounts]` e a foto do perfil em channels/<slug>/avatar*.jpg.
pub async fn login_on(ch: &Channel, platform: &str, timeout_min: u64) -> Result<String> {
    anyhow::ensure!(PLATFORMS.contains(&platform), "plataforma desconhecida: {platform}");
    let span = crate::events::step(&ch.slug, "login");
    span.action(&format!("Login no {}", if platform == "tiktok" { "TikTok" } else { "YouTube" }), "edge");
    let res: Result<LoginOut> = browser::run(
        "login.mjs",
        &json!({
            "profile": ch.slug,
            "timeoutMin": timeout_min,
            "platform": platform,
            "avatarOut": ch.avatar(platform).to_string_lossy(),
        }),
    )
    .await;
    let res = span.finish(res, |r| if r.ok { format!("@{}", r.handle) } else { r.error.clone() })?;
    anyhow::ensure!(res.ok, "login de {} no {platform} falhou: {}", ch.slug, res.error);

    // login novo: o autopilot volta a postar nessa rede
    if let Err(e) = crate::state::Store::open().and_then(|s| s.set_session(&ch.slug, platform, true)) {
        tracing::warn!("[{}] nao marquei a sessao do {platform} como valida: {e:#}", ch.slug);
    }
    if res.handle.is_empty() {
        tracing::warn!("[{}] sessao salva em {}, mas a conta nao foi detectada", ch.slug, res.dir);
    } else {
        set_account(ch, platform, &res.handle)?;
        tracing::info!("[{}] {platform} vinculado: @{}", ch.slug, res.handle);
    }
    Ok(res.handle)
}

/// Login no TikTok (compatibilidade com a CLI).
pub async fn login(ch: &Channel, timeout_min: u64) -> Result<String> {
    login_on(ch, "tiktok", timeout_min).await
}

/// Qual conta esta logada no perfil do canal (sem interacao).
pub async fn whoami(ch: &Channel) -> Result<WhoamiOut> {
    whoami_on(ch, "tiktok").await
}

pub async fn whoami_on(ch: &Channel, platform: &str) -> Result<WhoamiOut> {
    browser::run("whoami.mjs", &json!({ "profile": ch.slug, "platform": platform })).await
}

/// Grava a conta detectada em `[accounts]` (e em `post_handle`, para o TikTok).
pub fn set_account(ch: &Channel, platform: &str, handle: &str) -> Result<()> {
    let handle = handle.trim_start_matches('@').to_string();
    config::edit_channel(&ch.slug, |doc| {
        if platform == "tiktok" {
            doc["post_handle"] = toml_edit::value(handle.clone());
        }
        if !doc.contains_table("accounts") {
            doc["accounts"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        doc["accounts"][platform] = toml_edit::value(handle.clone());
        Ok(())
    })?;
    Ok(())
}

/// Sai de uma plataforma no perfil do canal: apaga so os cookies dela (o perfil guarda TikTok e Google juntos),
/// limpa o @ do channel.toml e a foto salva. A outra plataforma continua logada.
pub async fn logout_on(ch: &Channel, platform: &str) -> Result<()> {
    anyhow::ensure!(PLATFORMS.contains(&platform), "plataforma desconhecida: {platform}");
    #[derive(serde::Deserialize)]
    struct LogoutOut {
        ok: bool,
        #[serde(default)]
        error: String,
        #[serde(default)]
        removed: usize,
    }
    let span = crate::events::step(&ch.slug, "login");
    span.action(&format!("Saindo do {}", if platform == "tiktok" { "TikTok" } else { "YouTube" }), "edge");
    let res: Result<LogoutOut> =
        browser::run("logout.mjs", &json!({ "profile": ch.slug, "platform": platform })).await;
    let res = span.finish(res, |r| if r.ok { format!("{} cookies apagados", r.removed) } else { r.error.clone() })?;
    anyhow::ensure!(res.ok, "logout de {} no {platform} falhou: {}", ch.slug, res.error);
    set_account(ch, platform, "")?;
    let _ = std::fs::remove_file(ch.avatar(platform));
    tracing::info!("[{}] {platform} desconectado ({} cookies apagados)", ch.slug, res.removed);
    Ok(())
}

/// Grava o handle detectado de volta no channel.toml.
pub fn set_post_handle(ch: &Channel, handle: &str) -> Result<()> {
    set_account(ch, "tiktok", handle)
}

/// Lista o estado de login de cada canal.
pub async fn list(check: bool, only: Option<&str>) -> Result<()> {
    for ch in config::select(only)? {
        // diretorio de perfil existe nao significa sessao valida:
        // so o @handle gravado prova que o login foi ate o fim
        let profile_name = if has_profile(&ch.slug) { "dir ok" } else { "sem dir" };
        let account = if ch.post_handle.is_empty() {
            "SEM LOGIN".to_string()
        } else {
            format!("@{}", ch.post_handle)
        };
        if check && has_profile(&ch.slug) {
            match whoami(&ch).await {
                Ok(w) if w.logged_in => {
                    // handle recem-descoberto vai para o channel.toml
                    if ch.post_handle.is_empty() && !w.handle.is_empty() {
                        set_post_handle(&ch, &w.handle)?;
                        tracing::info!("[{}] conta vinculada: @{}", ch.slug, w.handle);
                    }
                    let handle_alert = if !ch.post_handle.is_empty()
                        && !w.handle.is_empty()
                        && w.handle.to_lowercase() != ch.post_handle.to_lowercase()
                    {
                        "  <-- DIVERGENTE do channel.toml"
                    } else {
                        ""
                    };
                    println!("{:<12} perfil: {profile_name}  conta: {account}  sessao: ativa (@{}){handle_alert}", ch.slug, w.handle);
                }
                Ok(_) => println!("{:<12} perfil: {profile_name}  conta: {account}  sessao: EXPIRADA", ch.slug),
                Err(e) => println!("{:<12} perfil: {profile_name}  conta: {account}  sessao: erro ({e})", ch.slug),
            }
        } else {
            println!("{:<12} perfil: {profile_name}  conta: {account}", ch.slug);
        }
    }
    Ok(())
}
