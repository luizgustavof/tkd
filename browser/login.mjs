import { open, isLoggedIn, whoami, isLoggedInYT, whoamiYT, avatarTikTok, saveImage, emit, log, evt, sleep, args } from "./lib.mjs";

// platform: "tiktok" | "youtube"; avatarOut: onde gravar a foto do perfil
const { profile = "default", timeoutMin = 10, platform = "tiktok", avatarOut = "" } = args();
const yt = platform === "youtube";
const logged = (ctx) => (yt ? isLoggedInYT(ctx) : isLoggedIn(ctx));

const { ctx, page, dir } = await open({ profile });

// YouTube: a conta Google pode ter varios canais (inclusive de marca). Mostra a pagina de troca
// de canal e espera a pessoa clicar no que esse canal do tkd vai usar.
const PICKING = /youtube\.com\/(account|channel_switcher|create_channel)|accounts\.google\./;

async function banner() {
  await page
    .evaluate(() => {
      if (document.getElementById("tkd-pick")) return;
      const d = document.createElement("div");
      d.id = "tkd-pick";
      d.style.cssText =
        "position:fixed;top:0;left:0;right:0;z-index:2147483647;display:flex;gap:16px;align-items:center;justify-content:center;padding:12px 20px;background:#8b8cff;color:#0e0f11;font:600 16px system-ui;box-shadow:0 2px 12px #0006";
      const t = document.createElement("span");
      t.textContent = "tkd: clique no canal do YouTube que vai receber os vídeos (o marcado com ✓ é o atual)";
      const b = document.createElement("button");
      b.textContent = "Usar o canal marcado";
      b.style.cssText = "font:600 15px system-ui;padding:8px 16px;border:0;border-radius:8px;background:#0e0f11;color:#fff;cursor:pointer";
      b.onclick = () => {
        window.__tkdPicked = true;
        b.textContent = "Salvando…";
      };
      d.append(t, b);
      document.documentElement.appendChild(d);
    })
    .catch(() => {});
}

async function pickChannelYT() {
  evt({ action: "Escolha o canal do YouTube na janela", progress: 70 });
  log("escolha o canal do YouTube na janela aberta...");
  await page.goto("https://www.youtube.com/channel_switcher", { waitUntil: "domcontentloaded" });
  const limit = Date.now() + timeoutMin * 60_000;
  while (Date.now() < limit) {
    if (page.isClosed()) throw new Error("janela fechada antes de escolher o canal");
    if (!PICKING.test(page.url())) return; // clicou num canal: o YouTube saiu da pagina de troca
    if (/\/(account|channel_switcher)/.test(page.url())) {
      if (await page.evaluate(() => window.__tkdPicked === true).catch(() => false)) return;
      await banner();
    }
    await sleep(1000);
  }
  throw new Error("tempo esgotado: nenhum canal do YouTube escolhido");
}

async function identify() {
  evt({ action: yt ? "Achando o canal do YouTube" : "Lendo o @ da conta", progress: 80 });
  if (yt) {
    await pickChannelYT();
    const me = await whoamiYT(page);
    const avatar = (await saveImage(ctx, me.avatar, avatarOut)) ? avatarOut : "";
    return { handle: me.handle || me.channelId, channelId: me.channelId, avatar };
  }
  const handle = await whoami(page);
  const url = handle ? await avatarTikTok(page, handle) : "";
  const avatar = (await saveImage(ctx, url, avatarOut)) ? avatarOut : "";
  return { handle, avatar };
}

try {
  if (await logged(ctx)) {
    const me = await identify();
    log(`perfil "${profile}" ja logado no ${platform} como @${me.handle}`);
    emit({ ok: true, profile, dir, platform, ...me, alreadyLoggedIn: true });
  } else {
    const url = yt
      ? "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F"
      : "https://www.tiktok.com/login";
    await page.goto(url, { waitUntil: "domcontentloaded" });
    evt({ action: "Esperando você entrar", progress: 20, url });
    log(`perfil "${profile}": faca login no ${platform} na janela aberta (ate ${timeoutMin} min)...`);

    let ok = false;
    for (let i = 0; i < timeoutMin * 30; i++) {
      if (await logged(ctx)) {
        ok = true;
        break;
      }
      if (i % 5 === 0) evt({ action: "Esperando você entrar", progress: 20 + (60 * i) / (timeoutMin * 30) });
      await sleep(2000);
    }
    if (!ok) {
      emit({ ok: false, profile, platform, error: "tempo esgotado: login nao detectado" });
    } else {
      await sleep(2500);
      const me = await identify();
      log(`perfil "${profile}" logado no ${platform} como @${me.handle}`);
      emit({ ok: true, profile, dir, platform, ...me, alreadyLoggedIn: false });
    }
  }
} catch (e) {
  emit({ ok: false, profile, platform, error: String(e) });
} finally {
  await ctx.close().catch(() => {});
}
