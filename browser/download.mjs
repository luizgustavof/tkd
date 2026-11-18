import fs from "node:fs";
import path from "node:path";
import { open, emit, log, human, args, waitForContent, sleep, evt } from "./lib.mjs";

const { videos = [], outDir, handle, profile = "default" } = args();
if (!outDir || !handle) {
  emit({ ok: false, error: "outDir e handle sao obrigatorios" });
  process.exit(0);
}
fs.mkdirSync(outDir, { recursive: true });

const GRID = `https://www.tiktok.com/@${handle}`;
const isMedia = (u) => /\/video\/tos\/|\.mp4/.test(u);

const { ctx, page } = await open({ profile });
const done = [];
const failed = [];

// a navegacao direta para /video/<id> e bloqueada (ERR_HTTP_RESPONSE_CODE_FAILURE);
// o caminho que funciona e abrir a grade e clicar no thumbnail
async function abrirGrade() {
  await page.goto(GRID, { waitUntil: "domcontentloaded" });
  await human(1500, 3000);
  return waitForContent(page, 'a[href*="/video/"]');
}

try {
  if (!(await abrirGrade())) {
    emit({ ok: false, error: "muro anti-bot na grade do perfil" });
    await ctx.close();
    process.exit(0);
  }

  let k = 0;
  for (const v of videos) {
    k++;
    evt({ action: `Baixando vídeo ${k} de ${videos.length}`, progress: ((k - 1) / videos.length) * 100, url: v.url });
    try {
      const alvo = page.locator(`a[href*="/video/${v.id}"]`).first();
      if ((await alvo.count()) === 0) {
        // o item pode estar abaixo da dobra
        await page.mouse.wheel(0, 1200);
        await human(800, 1600);
      }
      if ((await alvo.count()) === 0) {
        failed.push({ id: v.id, error: "thumbnail nao encontrado na grade" });
        continue;
      }

      // o <video> usa blob:; a URL real aparece na resposta de rede do CDN
      const esperaMidia = page
        .waitForResponse((r) => isMedia(r.url()) && r.status() < 400, { timeout: 45000 })
        .catch(() => null);

      await alvo.scrollIntoViewIfNeeded();
      await human(400, 1200);
      await alvo.click();
      const resp = await esperaMidia;
      await human(1500, 3000);

      if (!resp) {
        failed.push({ id: v.id, error: "player nao expos a midia" });
        await abrirGrade();
        continue;
      }
      const src = resp.url();

      const meta = await page.evaluate(() => {
        const t = (sel) => document.querySelector(sel)?.textContent?.trim() ?? "";
        return {
          caption: t('[data-e2e="browse-video-desc"]') || t('[data-e2e="video-desc"]'),
          music: t('[data-e2e="browse-music"]') || t('[data-e2e="video-music"]'),
          likes: t('[data-e2e="browse-like-count"]'),
          comments: t('[data-e2e="browse-comment-count"]'),
          shares: t('[data-e2e="browse-share-count"]'),
          hashtags: [...document.querySelectorAll('a[href*="/tag/"]')]
            .map((a) => a.textContent.trim())
            .filter(Boolean)
            .slice(0, 15),
        };
      });

      // baixa com os cookies e o referer da propria sessao
      const res = await ctx.request.get(src, {
        headers: { referer: page.url(), range: "bytes=0-" },
        timeout: 120000,
      });
      if (!res.ok() && res.status() !== 206) {
        failed.push({ id: v.id, error: `http ${res.status()}` });
        await abrirGrade();
        continue;
      }
      const file = path.join(outDir, `${v.id}.mp4`);
      fs.writeFileSync(file, await res.body());
      const mb = fs.statSync(file).size / 1e6;
      if (mb < 0.05) {
        fs.unlinkSync(file);
        failed.push({ id: v.id, error: "arquivo vazio" });
        await abrirGrade();
        continue;
      }

      done.push({ id: v.id, url: page.url(), file, ...meta });
      log(`baixado ${v.id} (${mb.toFixed(1)} MB)`);

      // fecha o player e volta para a grade
      await page.keyboard.press("Escape").catch(() => {});
      await sleep(800);
      if (!page.url().includes(`@${handle}`) || page.url().includes("/video/")) {
        await abrirGrade();
      }
      await human(1500, 3500);
    } catch (e) {
      failed.push({ id: v.id, error: String(e).split("\n")[0] });
      await abrirGrade().catch(() => {});
    }
  }

  emit({ ok: true, downloaded: done, failed });
} catch (e) {
  emit({ ok: false, error: String(e) });
} finally {
  await ctx.close();
}
