// Comentarios publicos de videos do TikTok, sem API key: abre o video no navegador e captura
// as respostas /api/comment/list/ que a propria pagina carrega.
// uso: node browser/comments.mjs --json '{"videos":[{"id","url"}],"outDir":"dataset/raw","max":30,"profile":"reader"}'
// saida: <outDir>/<id>.comments.json = [{cid, text, likes, replies, create_time, lang, dupes}]
//
// Para nao perder tempo nem gerar ruido:
// - um navegador so; cada video abre numa aba nova (fecha a anterior) — bem mais rapido que relancar o Edge;
// - para assim que a API diz has_more=0, quando a mesma pagina (cursor) volta repetida, ou apos 2 rolagens sem
//   comentario novo; prazo de 45 s por video;
// - comentarios repetidos (mesmo cid) contam uma vez; textos iguais de pessoas diferentes ("W edit", "🔥🔥")
//   viram um so, com `dupes` = quantas vezes apareceu e likes somados.
import fs from "node:fs";
import path from "node:path";
import { open, emit, log, human, args, evt } from "./lib.mjs";

const { videos = [], outDir = "dataset/raw", max = 30, profile = "reader", deadlineMs = 45000 } = args();
fs.mkdirSync(outDir, { recursive: true });
const result = [];

const norm = (t) => (t || "").toLowerCase().normalize("NFKC").replace(/\s+/g, " ").replace(/[!.?,;:]+$/g, "").trim();

function collapse(got) {
  const byText = new Map();
  for (const c of got.values()) {
    const k = norm(c.text);
    if (!k) continue;
    const prev = byText.get(k);
    if (prev) {
      prev.dupes += 1;
      prev.likes += c.likes;
      prev.replies += c.replies;
    } else byText.set(k, { ...c, dupes: 1 });
  }
  return [...byText.values()].sort((a, b) => b.likes - a.likes);
}

const pending = videos.filter((v) => !fs.existsSync(path.join(outDir, `${v.id}.comments.json`)));
for (const v of videos) if (!pending.includes(v)) result.push({ id: v.id, ok: true, cached: true });

let ctx = null;
let k = 0;
for (const v of pending) {
  k++;
  evt({ action: `Lendo comentários do vídeo ${k} de ${pending.length}`, progress: ((k - 1) / pending.length) * 100, url: v.url });
  const out = path.join(outDir, `${v.id}.comments.json`);
  const t0 = Date.now();
  if (!ctx) ctx = (await open({ profile })).ctx;
  const page = await ctx.newPage(); // abre a nova antes de fechar as velhas: sem abas o Edge persistente fecha
  for (const p of ctx.pages()) if (p !== page) await p.close().catch(() => {});
  const got = new Map();
  const cursors = new Set();
  let hasMore = true, repeats = 0, responses = 0;
  page.on("response", async (r) => {
    if (!/\/api\/comment\/list\//.test(r.url())) return;
    try {
      const j = await r.json();
      responses++;
      const cur = String(j.cursor ?? "");
      if (cursors.has(cur)) repeats++;
      cursors.add(cur);
      if (j.has_more === 0 || j.has_more === false) hasMore = false;
      for (const c of j.comments ?? []) {
        if (got.has(c.cid)) continue;
        got.set(c.cid, {
          cid: c.cid,
          text: c.text ?? c.share_info?.desc ?? "",
          likes: c.digg_count ?? 0,
          replies: c.reply_comment_total ?? 0,
          create_time: c.create_time,
          lang: c.comment_language ?? "",
        });
      }
    } catch {}
  });
  let body = "";
  try {
    await page.goto(v.url, { waitUntil: "domcontentloaded" });
    await human(2500, 3500);
    if (got.size === 0) {
      const btn = page.locator('[data-e2e="comment-icon"], button[aria-label*="omment"]').first();
      if ((await btn.count()) > 0) {
        await btn.click({ timeout: 3000 }).catch(() => {});
        await human(1500, 2500);
      }
    }
    let last = -1, still = 0;
    while (got.size < max && hasMore && still < 2 && repeats < 2 && Date.now() - t0 < deadlineMs) {
      const list = page.locator('[class*="CommentListContainer"], [data-e2e="comment-list"], [class*="DivCommentListContainer"]').first();
      if ((await list.count()) > 0 && got.size > 0) await list.hover({ timeout: 2000 }).catch(() => {});
      else await page.mouse.move(1050, 650);
      await page.mouse.wheel(0, 1400);
      await page.locator('[data-e2e="comment-level-1"]').last().scrollIntoViewIfNeeded({ timeout: 2000 }).catch(() => {});
      await human(900, 1500);
      still = got.size === last ? still + 1 : 0;
      last = got.size;
    }
    if (got.size === 0) body = (await page.evaluate(() => document.body.innerText.slice(0, 160))).replace(/\s+/g, " ");
  } catch (e) {
    body = String(e).slice(0, 160);
  }
  const arr = collapse(got);
  if (arr.length) fs.writeFileSync(out, JSON.stringify(arr, null, 1));
  const why = !hasMore ? "fim da lista" : repeats >= 2 ? "pagina repetida" : got.size >= max ? "max" : Date.now() - t0 >= deadlineMs ? "prazo" : "sem novos";
  const secs = ((Date.now() - t0) / 1000).toFixed(0);
  log(`${v.id}: ${arr.length} comentarios (${got.size} brutos, ${responses} respostas, parou: ${why}, ${secs}s) ${body}`);
  result.push({ id: v.id, ok: arr.length > 0, n: arr.length, raw: got.size, secs: Number(secs), stop: why, body });
  await human(1200, 2500); // a aba fica aberta ate a proxima ser criada
}
if (ctx) await ctx.close().catch(() => {});
emit({ ok: true, result });
