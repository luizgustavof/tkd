// Confere no perfil se videos ja sairam, antes de reenviar publicacoes que ficaram sem confirmacao
// (app/navegador morreu entre o upload e a gravacao no banco). So le: nunca clica em publicar.
// Uma abertura do navegador confere varios textos (ritmo anti-bloqueio do TikTok).
//   --json '{"platform":"tiktok","profile":"canal","texts":["legenda sem hashtags", "..."]}'
// Resposta: { ok, results: [{ state: "found" | "absent" | "unknown", url, note }] } (na ordem de `texts`)
//   found   = achou um post com esse texto (url do post, se deu para ler)
//   absent  = a lista de posts carregou e nenhum tem esse texto
//   unknown = nao deu para ter certeza (lista nao carregou/incompleta, layout mudou, sem sessao)
// Na duvida e "unknown": o chamador NAO reenvia sozinho.
import { open, emit, log, human, args, isLoggedIn, isLoggedInYT, whoamiYT, sleep, evt } from "./lib.mjs";

const { platform = "tiktok", profile = "default", text = "", texts: many = [] } = args();
const texts = many.length ? many : [text];

/** Texto comparavel: minusculo, sem acento, sem hashtags/mencoes, so letras e numeros. */
const norm = (s) =>
  (s || "")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[#@][\p{L}\p{N}_.]+/gu, " ")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();

// comeco curto: o Studio corta a legenda longa com "..."
const needle = (t) => norm(t).slice(0, 28).trim();

/** Linhas (texto + link) de cada post na lista; vazio = lista nao carregou. */
async function rows(page, linkSel) {
  return page.evaluate((sel) => {
    const out = [];
    for (const a of document.querySelectorAll(sel)) {
      let el = a;
      let t = "";
      // sobe ate o "card" do post: o primeiro ancestral com texto de verdade
      for (let i = 0; i < 8 && el; i++, el = el.parentElement) {
        t = (el.innerText || "").trim();
        if (t.length >= 12) break;
      }
      out.push({ href: a.href, text: t });
    }
    return out;
  }, linkSel);
}

/** Lista do TikTok Studio: { list, complete, note }. */
async function tiktokList(ctx, page) {
  if (!(await isLoggedIn(ctx))) return { list: [], complete: false, note: "perfil sem sessao do TikTok: conecte de novo" };
  evt({ action: "Conferindo se o vídeo já saiu no TikTok", progress: 20 });
  await page.goto("https://www.tiktok.com/tiktokstudio/content", { waitUntil: "domcontentloaded" });
  await human(3000, 5000);
  // "Publicações N" no topo: quantos posts a lista deveria ter
  const total = async () => {
    const body = await page.locator("body").innerText().catch(() => "");
    const m = body.match(/(?:Publica[cç][oõ]es|Posts)\s*\n?\s*(\d+)/i);
    return m ? Number(m[1]) : null;
  };
  let list = [];
  let n = null;
  for (let i = 0; i < 10; i++) {
    list = await rows(page, 'a[href*="/video/"]');
    n = await total();
    if (n === 0) return { list: [], complete: true, note: "conta sem posts" };
    if (list.length && n !== null && list.length >= Math.min(n, 10)) break;
    await sleep(1500);
  }
  if (!list.length) return { list, complete: false, note: "a lista de posts do TikTok Studio nao carregou" };
  // a publicacao sem confirmacao e das mais recentes: basta a lista ter carregado o topo (ate 10)
  const complete = n !== null && list.length >= Math.min(n, 10);
  return { list, complete, note: complete ? `${list.length} de ${n} posts conferidos` : `lista incompleta (${list.length} de ${n ?? "?"} posts)` };
}

/** Lista de Shorts do YouTube Studio: { list, complete, note }. */
async function youtubeList(ctx, page) {
  if (!(await isLoggedInYT(ctx))) return { list: [], complete: false, note: "perfil sem sessao do YouTube: conecte de novo" };
  evt({ action: "Conferindo se o vídeo já saiu no YouTube", progress: 20 });
  const me = await whoamiYT(page);
  if (!me.channelId) return { list: [], complete: false, note: "nao achei o canal do YouTube" };
  await page.goto(`https://studio.youtube.com/channel/${me.channelId}/videos/short`, { waitUntil: "domcontentloaded" });
  await human(3000, 5000);
  let list = [];
  for (let i = 0; i < 10 && !list.length; i++) {
    list = await rows(page, 'ytcp-video-row a[href*="/video/"]');
    if (!list.length) await sleep(1500);
  }
  if (!list.length) {
    const empty = await page.locator("ytcp-video-section-content #empty-state, .empty-state").first().isVisible().catch(() => false);
    return { list, complete: empty, note: empty ? "canal sem Shorts" : "a lista de Shorts do YouTube Studio nao carregou" };
  }
  return { list, complete: true, note: `${list.length} Shorts conferidos` };
}

function judge(t, { list, complete, note }) {
  const k = needle(t);
  if (k.length < 6) return { state: "unknown", url: "", note: "texto curto demais para reconhecer o post" };
  const hit = list.find((r) => norm(r.text).includes(k));
  if (!hit) return complete ? { state: "absent", url: "", note } : { state: "unknown", url: "", note };
  if (platform === "youtube") {
    const id = (hit.href.match(/\/video\/([\w-]{6,})/) || [])[1] || "";
    // rascunho (o YouTube cria um assim que o arquivo e escolhido) nao e video publicado
    if (/rascunho|draft/i.test(hit.text)) return { state: "absent", url: "", note: `so um rascunho (${id}) ficou no Studio` };
    return { state: "found", url: id ? `https://youtube.com/shorts/${id}` : hit.href, note: "" };
  }
  return { state: "found", url: hit.href, note: "" };
}

const { ctx, page } = await open({ profile });
try {
  const listing = platform === "youtube" ? await youtubeList(ctx, page) : await tiktokList(ctx, page);
  const results = texts.map((t) => judge(t, listing));
  log(`conferencia ${platform}: ${results.map((r) => r.state).join(", ")} (${listing.note})`);
  emit({ ok: true, results });
} catch (e) {
  emit({ ok: true, results: texts.map(() => ({ state: "unknown", url: "", note: String(e) })) });
} finally {
  await ctx.close().catch(() => {});
}
