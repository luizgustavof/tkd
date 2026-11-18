use crate::state::{Claim, Store};
use crate::{browser, config::Channel, render, review};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct UploadOut {
    ok: bool,
    #[serde(default)]
    handle: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    posted: bool,
    #[serde(default)]
    url: String,
    /// o script ja tinha clicado em publicar quando deu erro: pode ter saido
    #[serde(default)]
    committed: bool,
}

#[derive(Debug, Deserialize)]
struct CheckOut {
    #[serde(default)]
    results: Vec<CheckOne>,
}

#[derive(Debug, Deserialize)]
struct CheckOne {
    #[serde(default)]
    state: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    note: String,
}

/// Como terminou uma tentativa de upload.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// saiu e o script confirmou
    Posted { url: String, handle: String },
    /// nada saiu (erro antes de clicar em publicar): pode tentar de novo do zero
    Failed(String),
    /// sessao da rede caiu: definitivo, e o autopilot para de fazer video para essa rede ate o login
    Expired(String),
    /// nao da para saber se saiu (navegador morreu, estourou o tempo, erro depois do clique)
    Unknown(String),
}

/// Prefixo dos scripts para sessao caida: erro definitivo e com instrucao, nunca "tenta de novo".
const EXPIRED: &str = "SESSAO_EXPIRADA: ";

fn classify(res: Result<UploadOut>) -> Outcome {
    match res {
        Ok(r) if r.ok && r.posted => Outcome::Posted { url: r.url, handle: r.handle },
        Ok(r) if r.ok => Outcome::Unknown("upload terminou sem confirmar a publicacao".into()),
        Ok(r) if r.committed => Outcome::Unknown(format!("erro depois de clicar em publicar: {}", r.error)),
        Ok(r) if r.error.starts_with(EXPIRED) => Outcome::Expired(r.error.trim_start_matches(EXPIRED).to_string()),
        Ok(r) => Outcome::Failed(r.error),
        // o navegador morreu / estourou o tempo / nao devolveu JSON: o clique pode ter acontecido
        Err(e) => Outcome::Unknown(format!("{e:#}")),
    }
}

/// Resultado da conferencia do perfil.
#[derive(Debug, Clone, PartialEq)]
enum Check {
    Found(String),
    Absent(String),
    Unknown(String),
}

/// Limite diario por canal — protecao contra padrao de bot.
pub fn can_post(ch: &Channel, store: &Store) -> Result<bool> {
    let n = store.posts_last_24h(&ch.slug)?;
    Ok(n < ch.schedule.posts_per_day() as i64)
}

/// Redes em que o canal posta: as que tem conta conectada.
pub fn platforms(ch: &Channel) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !ch.post_handle.is_empty() || !ch.accounts.tiktok.is_empty() {
        out.push("tiktok");
    }
    if !ch.accounts.youtube.is_empty() {
        out.push("youtube");
    }
    out
}

/// Redes conectadas cuja sessao esta valida (nao caiu numa postagem desde o ultimo login).
pub fn live_platforms(ch: &Channel, store: &Store) -> Result<Vec<&'static str>> {
    let mut out = Vec::new();
    for n in platforms(ch) {
        if !store.session_expired(&ch.slug, n)? {
            out.push(n);
        }
    }
    Ok(out)
}

pub async fn publish(ch: &Channel, store: &Store, id: &str, dry_run: bool) -> Result<String> {
    let span = crate::events::step(&ch.slug, "publish");
    span.action(if dry_run { "Testando a postagem (sem publicar)" } else { "Postando" }, "edge");
    let r = publish_inner(ch, store, id, dry_run).await;
    span.finish(r, |u| u.clone())
}

/// Titulo do Short: o texto da legenda (sem hashtags), ate 100 caracteres.
fn yt_title(text: &str) -> String {
    let t = text.trim();
    let t = if t.is_empty() { "Short" } else { t };
    t.chars().take(100).collect()
}

/// Texto pelo qual o post e reconhecido no perfil (legenda sem hashtags / titulo do Short).
fn match_text(net: &str, post: &crate::hashtags::Post) -> String {
    if net == "youtube" {
        yt_title(&post.text)
    } else if post.text.trim().is_empty() {
        post.caption()
    } else {
        post.text.clone()
    }
}

/// Visibilidade do Short. Com o macaco do caos ligado, nunca publico (regra de ouro do docs/CHAOS.md).
fn yt_visibility(slug: &str) -> &'static str {
    if crate::chaos::enabled() || test_post_for(slug) {
        "UNLISTED"
    } else {
        "PUBLIC"
    }
}

/// Canais em post de teste agora (botao do app). Por canal: o piloto dos outros canais continua publico.
static TEST_POST_CHANNELS: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

/// Post de teste do canal: YouTube nao listado e TikTok "Somente voce". O script do TikTok confere a privacidade
/// escolhida e aborta sem publicar se nao conseguir marcar. Vale com `--test-post`/`TKD_TEST_POST=1` (CLI, o
/// processo inteiro) ou durante um `TestPost` do canal (app).
pub fn test_post_for(slug: &str) -> bool {
    std::env::var("TKD_TEST_POST").is_ok_and(|v| v == "1")
        || TEST_POST_CHANNELS.lock().map(|g| g.as_ref().is_some_and(|s| s.contains(slug))).unwrap_or(false)
}

/// Liga o post de teste de um canal enquanto o valor existir (desliga no drop, inclusive com erro/cancelamento).
pub struct TestPost(String);

impl TestPost {
    /// `None` se o canal ja esta num post de teste.
    pub fn begin(slug: &str) -> Option<Self> {
        let mut g = TEST_POST_CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(Default::default).insert(slug.to_string()).then(|| Self(slug.to_string()))
    }
}

impl Drop for TestPost {
    fn drop(&mut self) {
        if let Ok(mut g) = TEST_POST_CHANNELS.lock() {
            if let Some(s) = g.as_mut() {
                s.remove(&self.0);
            }
        }
    }
}

/// Intervalo minimo entre duas aberturas do navegador na mesma rede (TikTok mura o IP com aberturas
/// seguidas: 8 em 5 min ja bloqueou). Conferir e depois subir sao duas aberturas.
const OPEN_GAP: std::time::Duration = std::time::Duration::from_secs(45);

/// Espera o intervalo desde a ultima abertura nessa rede (deste processo) e marca a nova.
async fn pace(net: &str) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
    let wait = {
        let g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().and_then(|m| m.get(net)).map(|t| OPEN_GAP.saturating_sub(t.elapsed())).unwrap_or_default()
    };
    if !wait.is_zero() {
        let jitter = std::time::Duration::from_millis(rand::random::<u64>() % 15_000);
        tracing::info!("esperando {}s antes de abrir o {net} de novo (ritmo anti-bloqueio)", (wait + jitter).as_secs());
        tokio::time::sleep(wait + jitter).await;
    }
    LAST.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(net.to_string(), Instant::now());
}

/// Confere no perfil se os videos ja sairam (uma abertura do Studio para todos; nunca publica).
async fn check_posted(ch: &Channel, net: &str, texts: &[String]) -> Vec<Check> {
    pace(net).await;
    let r: Result<CheckOut> = browser::run(
        "check_post.mjs",
        &json!({ "platform": net, "profile": ch.slug, "texts": texts, "timeoutMin": 5 }),
    )
    .await;
    let mut results = match r {
        Ok(c) => c.results.into_iter().map(|c| match c.state.as_str() {
            "found" => Check::Found(c.url),
            "absent" => Check::Absent(c.note),
            _ => Check::Unknown(if c.note.is_empty() { "conferencia sem resposta".into() } else { c.note }),
        }).collect(),
        Err(e) => vec![Check::Unknown(format!("{e:#}")); texts.len()],
    };
    // resposta curta (script velho/quebrado) = sem certeza para os que faltaram
    results.resize(texts.len(), Check::Unknown("conferencia sem resposta".into()));
    results
}

/// Resolve as publicacoes deste canal que ficaram sem confirmacao (app/navegador morreu no meio):
/// confere o perfil e grava como postado o que ja esta no ar. O que nao esta fica liberado para
/// a proxima tentativa daquele video. Na duvida, fica como esta (e conta no limite do dia).
pub async fn reconcile(ch: &Channel, store: &Store, except: Option<&str>) -> Result<usize> {
    let mut fixed = 0;
    // o video que esta sendo publicado agora e conferido no proprio laco do publish
    let mut by_net: std::collections::BTreeMap<String, Vec<crate::state::Pending>> = Default::default();
    for p in store.unconfirmed_publishes(Some(&ch.slug))?.into_iter().filter(|p| Some(p.video_id.as_str()) != except) {
        // reserva o registro para ninguem mais mexer enquanto confere
        if let Claim::Unconfirmed { .. } = store.claim_publish(&p.channel, &p.platform, &p.video_id, &p.caption)? {
            by_net.entry(p.platform.clone()).or_default().push(p);
        }
    }
    for (net, items) in by_net {
        let texts: Vec<String> = items.iter().map(|p| p.caption.clone()).collect();
        for (p, check) in items.iter().zip(check_posted(ch, &net, &texts).await) {
            fixed += settle(ch, store, p, check)?;
        }
    }
    Ok(fixed)
}

/// Aplica o resultado da conferencia a uma publicacao reservada.
fn settle(ch: &Channel, store: &Store, p: &crate::state::Pending, check: Check) -> Result<usize> {
    match check {
        Check::Found(url) => {
            store.finish_publish(&p.channel, &p.platform, &p.video_id, &url, &p.caption)?;
            store.log(&ch.slug, "publish", "ok", &format!("{} {} (conferido no perfil) {url}", p.platform, p.video_id))?;
            tracing::warn!("[{}] {} ja estava no {}: gravado sem reenviar", ch.slug, p.video_id, p.platform);
            Ok(1)
        }
        Check::Absent(note) => {
            tracing::info!("[{}] {} nao saiu no {} ({note}): liberado para tentar de novo", ch.slug, p.video_id, p.platform);
            store.release_publish(&p.channel, &p.platform, &p.video_id)?;
            Ok(1)
        }
        Check::Unknown(why) => {
            store.mark_unconfirmed(&p.channel, &p.platform, &p.video_id, &p.detail)?;
            tracing::warn!("[{}] {} no {} segue sem confirmacao: {why}", ch.slug, p.video_id, p.platform);
            Ok(0)
        }
    }
}

/// Resolucao manual de uma publicacao sem confirmacao: `posted` grava como postado (com a URL), `retry` libera.
pub fn resolve(store: &Store, ch: &Channel, platform: &str, id: &str, posted_url: Option<&str>) -> Result<()> {
    let pending = store.unconfirmed_publishes(Some(&ch.slug))?;
    let p = pending
        .iter()
        .find(|p| p.platform == platform && p.video_id == id)
        .ok_or_else(|| anyhow::anyhow!("[{}] {id} no {platform} nao esta sem confirmacao", ch.slug))?;
    match posted_url {
        Some(url) => {
            store.finish_publish(&p.channel, platform, id, url, &p.caption)?;
            store.log(&ch.slug, "publish", "ok", &format!("{platform} {id} (marcado a mao) {url}"))?;
        }
        None => store.release_publish(&p.channel, platform, id)?,
    }
    Ok(())
}

async fn publish_inner(ch: &Channel, store: &Store, id: &str, dry_run: bool) -> Result<String> {
    anyhow::ensure!(
        review::approved(ch, id),
        "[{}] {id} nao esta na biblioteca (reprovado ou sem review)",
        ch.slug
    );
    anyhow::ensure!(
        crate::accounts::has_profile(&ch.slug),
        "[{}] sem perfil de navegador — conecte o TikTok ou o YouTube no app (ou `tkd login --channel {}`)",
        ch.slug,
        ch.slug
    );
    let nets = platforms(ch);
    anyhow::ensure!(!nets.is_empty(), "[{}] nenhuma rede conectada: conecte o TikTok ou o YouTube", ch.slug);
    if !dry_run {
        // regra de ouro do caos: nunca publicar de verdade com o macaco ligado. O YouTube sai nao listado;
        // o TikTok nao tem "nao listado" automatizado aqui, entao so em teste.
        anyhow::ensure!(
            !(crate::chaos::enabled() && nets.contains(&"tiktok")),
            "[{}] caos ligado (TKD_CHAOS): publicacao real no TikTok bloqueada; use --dry-run",
            ch.slug
        );
        // tentativas que morreram sem confirmar entram no limite do dia: resolve antes de contar
        if let Err(e) = reconcile(ch, store, Some(id)).await {
            tracing::warn!("[{}] conferencia das publicacoes pendentes falhou: {e:#}", ch.slug);
        }
    }
    // completar a rede que faltou (a outra ja postou esse video) nao e post novo
    let resuming = nets.iter().map(|n| store.posted_on(&ch.slug, n, id)).collect::<Result<Vec<_>>>()?.into_iter().any(|b| b);
    if !dry_run && !resuming {
        anyhow::ensure!(
            can_post(ch, store)?,
            "[{}] limite de {} post(s)/dia ja atingido (conta as publicacoes sem confirmacao)",
            ch.slug,
            ch.schedule.posts_per_day()
        );
    }

    let spec = render::load_spec(id)?;
    let file = ch.library_dir().join(id).join("out.mp4");

    let mut urls = Vec::new();
    let mut errors = Vec::new();
    let mut done = 0;
    for net in nets {
        let post = crate::hashtags::Post::new(&spec.caption, &spec.hashtags, &ch.hashtags, net);
        let caption = post.caption();
        let text = match_text(net, &post);
        if !dry_run {
            match store.claim_publish_within(&ch.slug, net, id, &text, Some(ch.schedule.posts_per_day() as i64))? {
                Claim::Fresh => {}
                Claim::LimitReached => {
                    errors.push(format!("{net}: limite de {} post(s)/dia ja atingido (outro processo postou antes)", ch.schedule.posts_per_day()));
                    continue;
                }
                Claim::AlreadyPosted => {
                    tracing::info!("[{}] {id} ja estava no {net}", ch.slug);
                    continue;
                }
                Claim::Busy { pid, since } => {
                    errors.push(format!("{net}: esse video ja esta sendo publicado (processo {pid}, desde {since})"));
                    continue;
                }
                Claim::Unconfirmed { since, detail } => {
                    // a tentativa anterior morreu sem confirmar: o video pode estar no ar. Reenviar as cegas duplicava.
                    tracing::warn!("[{}] {id} no {net}: tentativa de {since} sem confirmacao ({detail}); conferindo o perfil", ch.slug);
                    let check = check_posted(ch, net, std::slice::from_ref(&text)).await.pop();
                    match check.unwrap_or(Check::Unknown("conferencia sem resposta".into())) {
                        Check::Found(url) => {
                            store.finish_publish(&ch.slug, net, id, &url, &caption)?;
                            store.log(&ch.slug, "publish", "ok", &format!("{net} {url} (ja estava no ar)"))?;
                            done += 1;
                            if !url.is_empty() {
                                urls.push(url);
                            }
                            continue;
                        }
                        Check::Absent(note) => tracing::info!("[{}] {id} nao esta no {net} ({note}): enviando", ch.slug),
                        Check::Unknown(why) => {
                            store.mark_unconfirmed(&ch.slug, net, id, &detail)?;
                            let msg = format!(
                                "{net}: a tentativa de {since} ficou sem confirmacao e nao deu para conferir o perfil ({why}). \
                                 Confira no {net}: se saiu, `tkd publish --channel {} --id {id} --platform {net} --mark-posted <url>`; \
                                 se nao, `--retry-unconfirmed`",
                                ch.slug
                            );
                            store.log(&ch.slug, "publish", "erro", &msg)?;
                            errors.push(msg);
                            continue;
                        }
                    }
                }
            }
        }
        tracing::info!(
            "[{}] publicando {id} no {net}{}{}: {caption}",
            ch.slug,
            if dry_run { " (dry-run)" } else { "" },
            if test_post_for(&ch.slug) { " (POST DE TESTE: nao listado / somente voce)" } else { "" }
        );
        pace(net).await;
        let res: Result<UploadOut> = match net {
            "tiktok" => {
                browser::run(
                    "upload.mjs",
                    &json!({
                        "file": file.to_string_lossy(),
                        "caption": caption,
                        "dryRun": dry_run,
                        "profile": ch.slug,
                        "expectHandle": ch.post_handle,
                        "privacy": if test_post_for(&ch.slug) { "only_me" } else { "public" },
                    }),
                )
                .await
            }
            _ => {
                browser::run(
                    "upload_yt.mjs",
                    &json!({
                        "file": file.to_string_lossy(),
                        "title": yt_title(&post.text),
                        "description": caption,
                        "visibility": yt_visibility(&ch.slug),
                        "dryRun": dry_run,
                        "profile": ch.slug,
                        "expectHandle": ch.accounts.youtube,
                    }),
                )
                .await
            }
        };
        if dry_run {
            match res {
                Ok(r) if r.ok => {
                    store.log(&ch.slug, "publish", "dry-run", &format!("{net} {id} {}", r.url))?;
                    done += 1;
                    if !r.url.is_empty() {
                        urls.push(r.url);
                    }
                }
                Ok(r) => {
                    let e = r.error.trim_start_matches(EXPIRED).to_string();
                    store.log(&ch.slug, "publish", "erro", &format!("{net}: {e}"))?;
                    errors.push(format!("{net}: {e}"));
                }
                Err(e) => {
                    store.log(&ch.slug, "publish", "erro", &format!("{net}: {e:#}"))?;
                    errors.push(format!("{net}: {e:#}"));
                }
            }
            continue;
        }
        match classify(res) {
            Outcome::Posted { url, handle } => {
                // se o @ do TikTok so foi descoberto agora, registra no channel.toml
                if net == "tiktok" && ch.post_handle.is_empty() && !handle.is_empty() {
                    if let Err(e) = crate::accounts::set_post_handle(ch, &handle) {
                        tracing::warn!("[{}] nao gravei o @{handle}: {e:#}", ch.slug);
                    }
                }
                store.finish_publish(&ch.slug, net, id, &url, &caption)?;
                store.set_session(&ch.slug, net, true)?;
                store.log(&ch.slug, "publish", "ok", &format!("{net} {url}"))?;
                tracing::info!("[{}] publicado no {net}: {}", ch.slug, if url.is_empty() { "(url desconhecida)" } else { &url });
                done += 1;
                if !url.is_empty() {
                    urls.push(url);
                }
            }
            Outcome::Expired(e) => {
                store.release_publish(&ch.slug, net, id)?;
                store.set_session(&ch.slug, net, false)?;
                store.log(&ch.slug, "publish", "erro", &format!("{net}: {e}"))?;
                errors.push(format!("{net}: {e}"));
            }
            Outcome::Failed(e) => {
                store.release_publish(&ch.slug, net, id)?;
                store.log(&ch.slug, "publish", "erro", &format!("{net}: {e}"))?;
                errors.push(format!("{net}: {e}"));
            }
            Outcome::Unknown(e) => {
                store.mark_unconfirmed(&ch.slug, net, id, &e)?;
                let msg = format!("{net}: sem confirmacao ({e}); antes de reenviar o perfil vai ser conferido");
                store.log(&ch.slug, "publish", "erro", &msg)?;
                errors.push(msg);
            }
        }
    }

    // ao menos uma rede deu certo: sucesso (o erro da outra fica no historico)
    // (ou nada a fazer: o video ja estava em todas)
    if done > 0 || errors.is_empty() {
        for e in &errors {
            tracing::warn!("[{}] {e}", ch.slug);
        }
        return Ok(urls.join(" "));
    }
    anyhow::bail!("[{}] upload falhou: {}", ch.slug, errors.join(" | "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(ok: bool, posted: bool, committed: bool, error: &str) -> Result<UploadOut> {
        Ok(UploadOut { ok, handle: "h".into(), error: error.into(), posted, url: "u".into(), committed })
    }

    #[test]
    fn short_title() {
        assert_eq!(yt_title("Toji contra Gojo"), "Toji contra Gojo");
        assert_eq!(yt_title("  "), "Short");
        assert_eq!(yt_title(&"a".repeat(150)).chars().count(), 100);
    }

    #[test]
    fn only_final_failure_before_click() {
        assert_eq!(classify(out(true, true, true, "")), Outcome::Posted { url: "u".into(), handle: "h".into() });
        // erro antes de clicar em publicar: nada saiu
        assert_eq!(classify(out(false, false, false, "timeout no seletor")), Outcome::Failed("timeout no seletor".into()));
        // sessao caiu: definitivo e sem o prefixo tecnico
        assert_eq!(
            classify(out(false, false, false, "SESSAO_EXPIRADA: conecte de novo")),
            Outcome::Expired("conecte de novo".into())
        );
        // erro depois do clique, navegador morto, ok sem posted: nao da para saber
        assert!(matches!(classify(out(false, false, true, "nao confirmou")), Outcome::Unknown(_)));
        assert!(matches!(classify(out(true, false, false, "")), Outcome::Unknown(_)));
        assert!(matches!(classify(Err(anyhow::anyhow!("upload.mjs passou de 30 min"))), Outcome::Unknown(_)));
    }

    /// App morto entre o upload e o record_post, com o perfil de verdade (so LE o Studio; banco em memoria).
    /// `TKD_ROOT=<raiz> TKD_CHAOS_CHANNEL=<canal> TKD_CHAOS_POSTED="<legenda de um post que existe>"
    ///  cargo test -p tkd-core reconcile_ao_vivo -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn reconcile_live() {
        let ch = crate::config::get_channel(&std::env::var("TKD_CHAOS_CHANNEL").unwrap()).unwrap();
        let posted = std::env::var("TKD_CHAOS_POSTED").unwrap();
        let store = Store::memory().unwrap();
        // o upload saiu mas o app morreu antes de gravar: so a reserva ficou
        assert_eq!(store.claim_publish(&ch.slug, "tiktok", "sim-saiu", &posted).unwrap(), Claim::Fresh);
        store.mark_unconfirmed(&ch.slug, "tiktok", "sim-saiu", "app morto (simulado)").unwrap();
        // esse nunca saiu
        let never = "Texto que nunca foi postado zebra quantica 9931";
        assert_eq!(store.claim_publish(&ch.slug, "tiktok", "sim-nao-saiu", never).unwrap(), Claim::Fresh);
        store.mark_unconfirmed(&ch.slug, "tiktok", "sim-nao-saiu", "app morto (simulado)").unwrap();
        assert_eq!(store.posts_last_24h(&ch.slug).unwrap(), 2, "as duas contam no limite enquanto sem confirmacao");

        let fixed = reconcile(&ch, &store, None).await.unwrap();
        println!("resolvidas: {fixed}");
        assert!(store.posted_on(&ch.slug, "tiktok", "sim-saiu").unwrap(), "o que saiu vira post (sem reenviar)");
        assert!(!store.posted_on(&ch.slug, "tiktok", "sim-nao-saiu").unwrap());
        assert!(store.unconfirmed_publishes(None).unwrap().is_empty());
        assert_eq!(store.claim_publish(&ch.slug, "tiktok", "sim-nao-saiu", never).unwrap(), Claim::Fresh, "o que nao saiu fica livre");
        assert_eq!(store.claim_publish(&ch.slug, "tiktok", "sim-saiu", &posted).unwrap(), Claim::AlreadyPosted);
    }

    #[test]
    fn recognition_text() {
        let p = crate::hashtags::Post { text: "Toji contra Gojo".into(), tags: vec!["#anime".into()] };
        assert_eq!(match_text("tiktok", &p), "Toji contra Gojo");
        assert_eq!(match_text("youtube", &p), "Toji contra Gojo");
        let tags_only = crate::hashtags::Post { text: "".into(), tags: vec!["#anime".into()] };
        assert_eq!(match_text("tiktok", &tags_only), "#anime");
    }
}
