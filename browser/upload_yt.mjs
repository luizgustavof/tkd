// Posta um Short no YouTube pelo YouTube Studio, no perfil do canal (login feito com login.mjs).
// Vertical e ate 3 min: o YouTube classifica como Short sozinho.
//   --json '{"file":"out.mp4","title":"...","description":"...","visibility":"UNLISTED","profile":"_yt","expectHandle":"seu-canal","dryRun":true}'
// dryRun: faz tudo menos o "Salvar". O YouTube cria o video como rascunho PRIVADO assim que o
// arquivo e escolhido; no modo teste ele fica assim (ninguem ve) e o link volta na resposta.
import fs from "node:fs";
import { open, emit, log, human, args, isLoggedInYT, whoamiYT, sleep, evt } from "./lib.mjs";

const {
  file,
  title = "",
  description = "",
  visibility = "PUBLIC",
  dryRun = false,
  profile = "default",
  expectHandle = "",
} = args();

const VIS = ["PRIVATE", "UNLISTED", "PUBLIC"];
if (!file || !fs.existsSync(file)) {
  emit({ ok: false, error: `arquivo inexistente: ${file}` });
  process.exit(0);
}
if (!VIS.includes(visibility)) {
  emit({ ok: false, error: `visibility deve ser ${VIS.join(", ")}` });
  process.exit(0);
}

// titulo do YouTube: ate 100 caracteres, sem < e >
const cleanTitle = (title || "Short").replace(/[<>]/g, "").replace(/\s+/g, " ").trim().slice(0, 100);
const cleanDesc = description.replace(/[<>]/g, "").slice(0, 4900);

const { ctx, page } = await open({ profile });

/** Fecha os avisos que o Studio poe na frente (boas-vindas, novidades). */
async function dismissDialogs() {
  for (let i = 0; i < 4; i++) {
    const b = page
      .locator(
        'tp-yt-paper-dialog:visible button:has-text("Continuar"), tp-yt-paper-dialog:visible button:has-text("Continue"), ' +
          'ytcp-dialog:visible button:has-text("Entendi"), ytcp-dialog:visible button:has-text("Got it"), ' +
          'tp-yt-paper-dialog:visible #dismiss-button, tp-yt-paper-dialog:visible #close-button',
      )
      .first();
    if (!(await b.isVisible().catch(() => false))) return;
    await b.click().catch(() => {});
    await sleep(1200);
  }
}

/** Substitui o texto de um campo contenteditable do Studio. */
async function setText(sel, text) {
  const box = page.locator(sel).first();
  await box.waitFor({ state: "visible", timeout: 120000 });
  await box.click();
  await page.keyboard.press("Control+A");
  await page.keyboard.press("Backspace");
  for (const chunk of text.split(/(?<=\s)/)) {
    await page.keyboard.type(chunk, { delay: 25 + Math.random() * 45 });
    await sleep(60 + Math.random() * 160);
  }
}

// true a partir do "Salvar": dali em diante um erro NAO quer dizer "nao saiu" (ver upload.mjs)
let committed = false;

async function explain(e) {
  const expired = /accounts\.google\.com|ServiceLogin/.test(page.url()) || !(await isLoggedInYT(ctx).catch(() => true));
  if (expired) return `SESSAO_EXPIRADA: a sessao do YouTube caiu no meio do envio; conecte o YouTube de novo no app (perfil ${profile})`;
  return String(e);
}

async function main() {
  if (!(await isLoggedInYT(ctx))) {
    emit({ ok: false, error: `SESSAO_EXPIRADA: perfil "${profile}" sem sessao do YouTube; conecte o YouTube de novo no app (ou tkd login)` });
    return;
  }

  // trava de seguranca: nunca publicar no canal errado
  evt({ action: "Conferindo o canal do YouTube", progress: 5 });
  const me = await whoamiYT(page);
  const handle = me.handle || me.channelId;
  const want = expectHandle.replace(/^@/, "").toLowerCase();
  if (want && handle && ![me.handle.toLowerCase(), me.channelId.toLowerCase()].includes(want)) {
    emit({ ok: false, error: `canal errado: perfil "${profile}" esta no @${handle}, esperado @${want}`, handle });
    return;
  }
  log(`postando no YouTube como @${handle || "?"} (perfil ${profile}, ${visibility})`);

  evt({ action: "Abrindo o YouTube Studio", progress: 10, url: "https://studio.youtube.com" });
  await page.goto("https://studio.youtube.com", { waitUntil: "domcontentloaded" });
  await human(3000, 5000);
  await dismissDialogs();

  await page.locator("#upload-icon").click();
  const input = await page.waitForSelector('input[type="file"]', { state: "attached", timeout: 60000 });
  await input.setInputFiles(file);
  log("arquivo escolhido, enviando...");
  evt({ action: "Enviando o vídeo", progress: 25 });

  // o link sai logo que o YouTube cria o video (ja privado)
  let url = "";
  for (let i = 0; i < 60 && !url; i++) {
    url = (await page.locator("ytcp-video-info a").first().getAttribute("href").catch(() => "")) || "";
    if (!url) await sleep(1000);
  }

  evt({ action: "Escrevendo título e descrição", progress: 40 });
  await setText("#title-textarea #textbox", cleanTitle);
  await human(500, 1200);
  if (cleanDesc) await setText("#description-textarea #textbox", cleanDesc);
  await human(500, 1200);

  // obrigatorio: "nao e conteudo para criancas"
  const notKids = page.locator('tp-yt-paper-radio-button[name="VIDEO_MADE_FOR_KIDS_NOT_MFK"]');
  await notKids.scrollIntoViewIfNeeded();
  await notKids.click();
  await human(600, 1400);

  // Detalhes -> Elementos -> Verificacoes -> Visibilidade
  for (let i = 0; i < 3; i++) {
    await page.locator("#next-button").click();
    await human(1500, 2500);
  }
  evt({ action: "Escolhendo a visibilidade", progress: 70 });
  await page.locator(`tp-yt-paper-radio-button[name="${visibility}"]`).click();
  await human(800, 1500);

  // espera o envio do arquivo terminar (o processamento continua no servidor depois)
  evt({ action: "Esperando o envio terminar", progress: 80 });
  // prazo pelo relogio: cada leitura sem timeout proprio esperava ate 30 s quando o elemento sumia, e os
  // "10 min" viravam horas (segurando a trava do navegador de todos os canais)
  let enviado = false;
  const deadline = Date.now() + 10 * 60_000;
  while (Date.now() < deadline) {
    const dialogOpen = await page.locator("ytcp-uploads-dialog").isVisible().catch(() => false);
    if (!dialogOpen) {
      // o dialogo fechou sem o nosso clique (publicado a mao, ou o Studio fechou): pode ter saido.
      // committed = o tkd confere o canal antes de reenviar, em vez de subir de novo (video duplicado)
      committed = true;
      throw new Error("o dialogo de envio do YouTube fechou antes do Salvar (publicado a mao?)");
    }
    const t = await page.locator("ytcp-video-upload-progress").innerText({ timeout: 3000 }).catch(() => "");
    // "Upload concluido" pode vir junto de "Processando 45%": o processamento segue no servidor e nao impede salvar
    const uploaded = /upload (conclu|complete)|envio conclu|carregamento conclu/i.test(t);
    const settled = /conclu|complete|processamento|processing|verifica|checks/i.test(t) && !/\d+\s?%/.test(t);
    if (uploaded || settled) {
      enviado = true;
      break;
    }
    await sleep(2000);
  }
  if (!enviado) throw new Error("o envio do arquivo nao terminou em 10 min");

  if (dryRun) {
    emit({
      ok: true,
      dryRun: true,
      posted: false,
      handle,
      url,
      note: "pronto pra salvar; Salvar nao clicado (o YouTube deixa o video como rascunho privado)",
    });
    return;
  }

  committed = true;
  await page.locator("#done-button").click();
  log("salvo, conferindo...");
  evt({ action: "Conferindo a publicação", progress: 92 });
  // confirmacao: dialogo de compartilhar (publico/nao listado) ou o dialogo de envio fecha
  for (let i = 0; i < 30; i++) {
    const shared = await page.locator("ytcp-video-share-dialog, ytcp-uploads-still-processing-dialog").first().isVisible().catch(() => false);
    const open_ = await page.locator("ytcp-uploads-dialog #done-button").isVisible().catch(() => false);
    if (shared || !open_) break;
    await sleep(1000);
  }
  emit({ ok: true, posted: true, url, handle, visibility });
}

try {
  await main();
} catch (e) {
  emit({ ok: false, error: await explain(e), committed });
} finally {
  await sleep(1500);
  await ctx.close().catch(() => {});
}
