import { open, emit, log, human, humanScroll, waitForContent, isLoggedIn, args, evt } from "./lib.mjs";

const { handle, limit = 20, profile = "default" } = args();

if (!handle) {
  emit({ ok: false, error: "handle obrigatorio" });
  process.exit(0);
}

const { ctx, page } = await open({ profile });
try {
  if (!(await isLoggedIn(ctx))) log("AVISO: sem sessao. rode `tkd login` primeiro.");

  const url = `https://www.tiktok.com/@${handle}`;
  evt({ action: `Olhando o perfil @${handle}`, url });
  await page.goto(url, { waitUntil: "domcontentloaded" });
  await human(1500, 3000);

  const got = await waitForContent(page, 'a[href*="/video/"]');
  if (!got) {
    emit({ ok: false, error: "muro anti-bot ou perfil sem videos", url });
    await ctx.close();
    process.exit(0);
  }

  evt({ action: `Rolando a grade de @${handle}` });
  await humanScroll(page, Math.ceil(limit / 8) + 2);

  const account = await page.evaluate(() => {
    const t = (sel) => document.querySelector(sel)?.textContent?.trim() ?? "";
    return {
      nickname: t('[data-e2e="user-subtitle"]') || t("h2"),
      signature: t('[data-e2e="user-bio"]'),
      followers: t('[data-e2e="followers-count"]'),
      likes: t('[data-e2e="likes-count"]'),
    };
  });

  // posts de foto (carrossel) vivem em /photo/<id> — contados, nunca baixados
  const photos = await page.evaluate(
    () => new Set([...document.querySelectorAll('a[href*="/photo/"]')].map((a) => a.href)).size
  );

  const videos = await page.evaluate(() => {
    const out = [];
    const seen = new Set();
    for (const a of document.querySelectorAll('a[href*="/video/"]')) {
      const href = a.href;
      const id = href.split("/video/")[1]?.split(/[?#]/)[0];
      if (!id || seen.has(id)) continue;
      seen.add(id);
      const card = a.closest("div");
      out.push({
        id,
        url: href,
        caption: a.querySelector("img")?.alt ?? "",
        views: card?.querySelector('[data-e2e="video-views"]')?.textContent?.trim() ?? "",
        cover: a.querySelector("img")?.src ?? "",
      });
    }
    return out;
  });

  emit({ ok: true, handle, account, photos, videos: videos.slice(0, limit) });
} catch (e) {
  emit({ ok: false, error: String(e) });
} finally {
  await ctx.close();
}
