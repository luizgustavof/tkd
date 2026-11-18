import fs from "node:fs";
import { open, emit, log, human, args, isLoggedIn, whoami, sleep, evt } from "./lib.mjs";

const {
  file,
  caption = "",
  dryRun = false,
  profile = "default",
  expectHandle = "",
  // "only_me" = post de teste ("Somente voce"); confere e aborta sem publicar se nao conseguir marcar
  privacy = "public",
} = args();

const ONLY_ME = [/somente (voc[eê]|eu)/i, /^s[oó] (voc[eê]|eu)$/i, /only (you|me)/i, /^privad/i, /^private/i];

/** Marca "Somente voce" em "Quem pode ver esta publicacao" e devolve o texto que ficou no seletor. */
async function setOnlyMe() {
  const box = page.locator('[data-e2e="video_visibility_container"]').first();
  await box.scrollIntoViewIfNeeded({ timeout: 15000 });
  const trigger = box.locator('[role="combobox"]').first();
  for (let attempt = 0; attempt < 3; attempt++) {
    await trigger.click();
    await sleep(900);
    // as opcoes abrem num popup fora do container: pega o elemento visivel mais interno com o texto
    const picked = await page.evaluate((sources) => {
      const res = sources.map((s) => new RegExp(s, "i"));
      const vis = (el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
      const all = [...document.querySelectorAll('[role="option"], [role="menuitem"], [role="radio"], li, label, div, span')];
      const hit = all.filter((el) => vis(el) && !el.closest('[data-e2e="video_visibility_container"]'))
        .filter((el) => res.some((r) => r.test((el.innerText || "").trim())) && (el.innerText || "").trim().length < 40)
        .sort((a, b) => a.querySelectorAll("*").length - b.querySelectorAll("*").length)[0];
      if (!hit) return null;
      (hit.closest('[role="option"], [role="menuitem"], [role="radio"], li, label') || hit).click();
      return (hit.innerText || "").trim();
    }, ONLY_ME.map((r) => r.source));
    await sleep(900);
    const now = (await trigger.innerText().catch(() => "")).trim();
    if (ONLY_ME.some((r) => r.test(now))) {
      log(`privacidade: "${now}" (opcao clicada: "${picked}")`);
      return now;
    }
    await page.keyboard.press("Escape").catch(() => {});
    await sleep(700);
  }
  throw new Error(`nao consegui marcar "Somente voce" em "Quem pode ver" (o seletor continua "${(await trigger.innerText().catch(() => "?")).trim()}"): nada publicado`);
}

if (!file || !fs.existsSync(file)) {
  emit({ ok: false, error: `arquivo inexistente: ${file}` });
  process.exit(0);
}

const { ctx, page, dir } = await open({ profile });

// Avisos que o TikTok Studio poe na frente (uma vez por conta ou de vez em quando).
// Verificacao automatica de conteudo: "Ativar" (avisa antes se a musica vai ser silenciada).
const MODAL_BUTTONS = ["Ativar", "Turn on", "Publicar agora", "Postar agora", "Post now", "Entendi", "Got it", "OK", "Continuar", "Continue", "Fechar", "Close"];
async function dismissTour() {
  // tour de boas-vindas (react-joyride): pular/fechar; senao avanca ate acabar
  let acted = false;
  for (let i = 0; i < 8; i++) {
    if (!(await page.locator("#react-joyride-portal .react-joyride__overlay, .react-joyride__tooltip").first().isVisible().catch(() => false))) break;
    acted = true;
    const b = page.locator('[data-test-id="button-skip"], [data-test-id="button-close"], [data-test-id="button-primary"]').first();
    if (await b.isVisible().catch(() => false)) {
      log(`tour do TikTok: "${(await b.innerText().catch(() => "")).trim() || "fechar"}"`);
      await b.click({ force: true }).catch(() => {});
    } else await page.keyboard.press("Escape");
    await sleep(800);
  }
  return acted;
}

async function dismissDialogs() {
  let acted = false;
  for (let i = 0; i < 4; i++) {
    const modal = page.locator('.TUXModal-overlay [role="dialog"], .TUXModal-overlay').first();
    if (!(await modal.isVisible().catch(() => false))) break;
    acted = true;
    let clicked = false;
    for (const name of MODAL_BUTTONS) {
      const b = modal.getByRole("button", { name, exact: true }).first();
      if (await b.isVisible().catch(() => false)) {
        log(`aviso do TikTok: "${name}"`);
        await b.click().catch(() => {});
        clicked = true;
        break;
      }
    }
    if (!clicked) await page.keyboard.press("Escape");
    await sleep(1200);
  }
  return acted;
}

// Conta nova: o aviso "Ativar" fecha e o tour de boas-vindas abre LOGO DEPOIS (antes o tour era tratado so
// uma vez, antes do aviso, ficava na frente e o clique na legenda morria por timeout: nada publicava).
// Repete ate uma rodada inteira sem nada na tela.
async function dismissModals() {
  for (let round = 0; round < 6; round++) {
    const a = await dismissTour();
    const b = await dismissDialogs();
    if (!a && !b) return;
    await sleep(1500); // o proximo aviso/tour demora um pouco para aparecer
  }
}

// true a partir do clique em "Publicar": dali em diante um erro NAO quer dizer "nao saiu" (o TikTok
// pode ter recebido). O tkd marca a publicacao como sem confirmacao e confere o perfil antes de reenviar.
let committed = false;

/** Erro legivel: sessao que caiu no meio vira "conecte de novo" (e o tkd nao tenta em loop). */
async function explain(e) {
  const expired = /\/login/.test(page.url()) || !(await isLoggedIn(ctx).catch(() => true));
  if (expired) return `SESSAO_EXPIRADA: a sessao do TikTok caiu no meio do envio; conecte o TikTok de novo no app (perfil ${profile})`;
  return String(e);
}

try {
  if (!(await isLoggedIn(ctx))) {
    emit({ ok: false, error: `SESSAO_EXPIRADA: perfil "${profile}" sem sessao do TikTok; conecte o TikTok de novo no app (ou \`tkd login --channel ${profile}\`)` });
    await ctx.close();
    process.exit(0);
  }

  // trava de seguranca: nunca publicar na conta errada
  const handle = await whoami(page);
  if (expectHandle && handle && handle.toLowerCase() !== expectHandle.toLowerCase()) {
    emit({
      ok: false,
      error: `conta errada: perfil "${profile}" esta logado como @${handle}, esperado @${expectHandle}`,
      handle,
    });
    await ctx.close();
    process.exit(0);
  }
  log(`publicando como @${handle || "?"} (perfil ${profile})`);

  evt({ action: "Abrindo o TikTok Studio", progress: 5, url: "https://www.tiktok.com/tiktokstudio/upload" });
  await page.goto("https://www.tiktok.com/tiktokstudio/upload?from=upload", {
    waitUntil: "domcontentloaded",
  });
  await human(2000, 4000);

  // o input de arquivo costuma estar escondido atras do dropzone
  const input = await page.waitForSelector('input[type="file"]', {
    state: "attached",
    timeout: 60000,
  });
  await input.setInputFiles(file);
  log("arquivo enviado, aguardando processamento...");
  evt({ action: "Enviando o vídeo", progress: 30 });

  // campo de caption (DraftJS contenteditable)
  const editor = await page.waitForSelector('div[contenteditable="true"]', { timeout: 120000 });
  await human(1500, 3000);
  await dismissModals();
  await editor.click();
  // limpa o texto que o TikTok preenche com o nome do arquivo
  await page.keyboard.press("Control+A");
  await page.keyboard.press("Backspace");
  evt({ action: "Escrevendo a legenda", progress: 50 });
  for (const chunk of caption.split(" ")) {
    await page.keyboard.type(chunk + " ", { delay: 40 + Math.random() * 60 });
    // hashtag abre um dropdown de sugestao: Escape evita selecionar outra tag
    if (chunk.startsWith("#")) {
      await sleep(700);
      await page.keyboard.press("Escape");
    }
    await sleep(120 + Math.random() * 280);
  }

  let visibility = "Todos";
  if (privacy === "only_me") {
    await dismissModals();
    visibility = await setOnlyMe();
  }

  // espera o upload terminar: o botao de publicar sai do estado desabilitado
  const postBtn = page
    .locator('[data-e2e="post_video_button"], button:has-text("Publicar"), button:has-text("Post")')
    .first();
  for (let i = 0; i < 120; i++) {
    if (i % 5 === 0) await dismissModals();
    const disabled = await postBtn.isDisabled().catch(() => true);
    if (!disabled) break;
    await sleep(2000);
  }

  if (dryRun) {
    emit({
      ok: true,
      dryRun: true,
      posted: false,
      handle,
      visibility,
      note: "pronto para publicar; botao nao clicado",
    });
    await sleep(3000);
    await ctx.close();
    process.exit(0);
  }

  await human(800, 2000);
  await dismissModals();
  if (privacy === "only_me") {
    // modal ou rolagem podem ter mexido no seletor: confere de novo imediatamente antes do clique
    const now = (await page.locator('[data-e2e="video_visibility_container"] [role="combobox"]').first().innerText().catch(() => "")).trim();
    if (!ONLY_ME.some((r) => r.test(now))) throw new Error(`privacidade mudou para "${now}" antes de publicar: nada publicado`);
  }
  committed = true;
  await postBtn.click();
  log("clicado em publicar, aguardando confirmacao...");
  evt({ action: "Conferindo a publicação", progress: 90 });
  // confirma de verdade: o Studio vai pra lista de conteudo ou mostra "publicado"
  let confirmed = false;
  for (let i = 0; i < 45 && !confirmed; i++) {
    await sleep(2000);
    await dismissModals();
    if (/tiktokstudio\/content|\/manage/.test(page.url())) confirmed = true;
    else {
      const t = await page.locator("body").innerText().catch(() => "");
      if (/(v[ií]deo|post) (foi )?(publicad|postad)|your video (has been|is being) (posted|uploaded)|video published/i.test(t)) confirmed = true;
    }
  }
  if (!confirmed) {
    await page.screenshot({ path: `${dir}/upload_falhou.png` }).catch(() => {});
    throw new Error("cliquei em publicar mas o TikTok nao confirmou (print em upload_falhou.png no perfil)");
  }

  // tenta descobrir a URL do post recem-criado
  let url = "";
  try {
    await page.goto("https://www.tiktok.com/tiktokstudio/content", {
      waitUntil: "domcontentloaded",
    });
    await human(2500, 4500);
    url = await page.evaluate(() => {
      const a = document.querySelector('a[href*="/video/"]');
      return a ? a.href : "";
    });
  } catch {}

  emit({ ok: true, posted: true, url, handle });
} catch (e) {
  emit({ ok: false, error: await explain(e), committed });
} finally {
  await ctx.close().catch(() => {});
}
