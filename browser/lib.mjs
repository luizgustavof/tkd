import { chromium } from "playwright";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
/** Um diretorio de perfil por canal: .profiles/<slug>/ */
export const PROFILES = path.join(ROOT, ".profiles");
export const profileDir = (name) => path.join(PROFILES, name || "default");

/** Payload JSON vem em --json '<...>'. */
export function args() {
  const i = process.argv.indexOf("--json");
  if (i === -1) return {};
  return JSON.parse(process.argv[i + 1] ?? "{}");
}

/** Resultado do script: UMA linha JSON no stdout. Logs vao para stderr. */
export function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}
/** Evento ao vivo para o app: { action?, progress?, url? } numa linha JSON do stderr. */
export function evt(o) {
  process.stderr.write(JSON.stringify({ evt: 1, ...o }) + "\n");
}
export function log(...a) {
  console.error("[browser]", ...a);
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/** Pausa humana aleatoria. */
export const human = (min = 400, max = 1800) =>
  sleep(min + Math.floor(Math.random() * (max - min)));

/**
 * Contexto persistente da conta `profile`.
 * headless sempre false: headless e o que dispara o muro anti-bot do TikTok.
 */
export async function open({ profile = "default", headless = false, hidden = false } = {}) {
  const dir = profileDir(profile);
  const ctx = await chromium.launchPersistentContext(dir, {
    headless,
    channel: "msedge",
    viewport: { width: 1280, height: 900 },
    locale: "pt-BR",
    timezoneId: "America/Sao_Paulo",
    args: [
      "--disable-blink-features=AutomationControlled",
      // hidden: janela de verdade (nao headless), mas fora da tela; para conferencias rapidas
      ...(hidden ? ["--window-position=-32000,-32000", "--start-minimized"] : []),
    ],
  });
  ctx.setDefaultTimeout(45000);
  const page = ctx.pages()[0] ?? (await ctx.newPage());
  return { ctx, page, dir };
}

export async function isLoggedIn(ctx) {
  const cookies = await ctx.cookies("https://www.tiktok.com");
  return cookies.some((c) => c.name === "sessionid" && c.value);
}

/** Qual conta esta logada neste perfil (@handle), ou "" se nenhuma. */
export async function whoami(page) {
  // /profile redireciona o usuario logado para o proprio /@handle
  try {
    await page.goto("https://www.tiktok.com/profile", { waitUntil: "domcontentloaded" });
    await human(1500, 3000);
    const m = page.url().match(/tiktok\.com\/@([\w.\-]+)/);
    if (m) return m[1];
    const html = await page.content();
    const m2 = html.match(/"uniqueId":"([\w.\-]+)"/);
    if (m2) return m2[1];
  } catch {}
  // fallback: o painel do Studio lista o proprio perfil nos links
  try {
    await page.goto("https://www.tiktok.com/tiktokstudio", { waitUntil: "domcontentloaded" });
    await human(1500, 3000);
    const href = await page.evaluate(() => {
      const a = document.querySelector('a[href*="/@"]');
      return a?.getAttribute("href") ?? "";
    });
    const m3 = href.match(/\/@([\w.\-]+)/);
    if (m3) return m3[1];
  } catch {}
  return "";
}

/** Espera o conteudo real aparecer; detecta o muro "Please wait" / captcha. */
export async function waitForContent(page, selector, { tries = 3 } = {}) {
  for (let i = 0; i < tries; i++) {
    try {
      await page.waitForSelector(selector, { timeout: 20000 });
      return true;
    } catch {
      const text = await page.evaluate(() => document.body.innerText.slice(0, 200));
      log(`tentativa ${i + 1}: sem '${selector}'. body="${text.replace(/\s+/g, " ").trim()}"`);
      await human(2500, 5000);
      await page.reload({ waitUntil: "domcontentloaded" }).catch(() => {});
    }
  }
  return false;
}

/** Rolagem com inercia, parecida com a humana. */
export async function humanScroll(page, steps = 6) {
  for (let i = 0; i < steps; i++) {
    await page.mouse.wheel(0, 600 + Math.floor(Math.random() * 700));
    await human(500, 1400);
  }
}

// ------------------------------------------------------------- YouTube

export async function isLoggedInYT(ctx) {
  const cookies = await ctx.cookies("https://www.youtube.com");
  return cookies.some((c) => (c.name === "LOGIN_INFO" || c.name === "SAPISID") && c.value);
}

/** Canal do YouTube logado: { handle, channelId, avatar } (vazios se nao achar). */
export async function whoamiYT(page) {
  const out = { handle: "", channelId: "", avatar: "" };
  try {
    // o Studio redireciona para /channel/UC... do canal da conta
    await page.goto("https://studio.youtube.com", { waitUntil: "domcontentloaded" });
    await human(2500, 4000);
    const m = page.url().match(/\/channel\/(UC[\w-]+)/);
    if (m) out.channelId = m[1];
  } catch {}
  if (!out.channelId) return out;
  try {
    await page.goto(`https://www.youtube.com/channel/${out.channelId}`, { waitUntil: "domcontentloaded" });
    await human(1500, 3000);
    const html = await page.content();
    const h = html.match(/"canonicalBaseUrl":"\/@([^"]+)"/) || page.url().match(/youtube\.com\/@([^/?]+)/);
    if (h) out.handle = decodeURIComponent(h[1]);
    out.avatar = await page.evaluate(() => document.querySelector('meta[property="og:image"]')?.content ?? "");
  } catch {}
  // logo depois de trocar de canal a pagina logada as vezes vem sem o @: le a publica
  if (!out.handle || !out.avatar) {
    try {
      const html = await getPublic(`https://www.youtube.com/channel/${out.channelId}`, "pt-BR,pt;q=0.9");
      const h = html.match(/"canonicalBaseUrl":"\/@([^"]+)"/) || html.match(/"vanityChannelUrl":"https?:\/\/www\.youtube\.com\/@([^"]+)"/);
      if (!out.handle && h) out.handle = decodeURIComponent(h[1]);
      const img = html.match(/<meta property="og:image" content="([^"]+)"/);
      if (!out.avatar && img) out.avatar = img[1];
    } catch {}
  }
  return out;
}

/**
 * GET de pagina publica com TLS de Chrome (scripts/web.py, curl_cffi do Scrapling), sem cookies.
 * Sem o venv, cai no fetch() do Node (impressao digital TLS de Node, mais facil de barrar).
 */
export async function getPublic(url, lang = "pt-BR,pt;q=0.9") {
  const { execFile } = await import("node:child_process");
  const fs = await import("node:fs");
  const venv = path.join(ROOT, "tools", "rea-video", ".venv");
  const py = [path.join(venv, "Scripts", "python.exe"), path.join(venv, "bin", "python")].find((p) => fs.existsSync(p));
  if (py) {
    const html = await new Promise((res) =>
      execFile(py, ["-I", "-X", "utf8", path.join(ROOT, "scripts", "web.py"), "get", url, lang],
        { maxBuffer: 32 << 20, windowsHide: true, timeout: 30000 },
        (err, stdout) => res(err ? null : stdout))
    );
    if (html) return html;
  }
  const r = await fetch(url, { headers: { "User-Agent": "Mozilla/5.0", "Accept-Language": lang } });
  return await r.text();
}

/** Foto do perfil do TikTok logado (url), lida da pagina /@handle. */
export async function avatarTikTok(page, handle) {
  try {
    if (!page.url().includes(`/@${handle}`)) {
      await page.goto(`https://www.tiktok.com/@${handle}`, { waitUntil: "domcontentloaded" });
      await human(1500, 2500);
    }
    const html = await page.content();
    const m = html.match(/"avatarLarger":"([^"]+)"/) || html.match(/"avatarMedium":"([^"]+)"/);
    if (m) return JSON.parse(`"${m[1]}"`);
    return await page.evaluate(() => document.querySelector('meta[property="og:image"]')?.content ?? "");
  } catch {
    return "";
  }
}

/** Baixa a imagem com os cookies do perfil e grava em `file`. */
export async function saveImage(ctx, url, file) {
  if (!url || !file) return false;
  try {
    const fs = await import("node:fs");
    const r = await ctx.request.get(url);
    if (!r.ok()) return false;
    fs.writeFileSync(file, await r.body());
    return true;
  } catch {
    return false;
  }
}
