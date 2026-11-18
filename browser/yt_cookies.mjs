// Cookies do YouTube para o yt-dlp, de uma conta Google descartavel (nunca a conta de um canal).
// Perfil proprio do Edge: .profiles/_youtube/. Saida: .profiles/_youtube/cookies.txt (formato Netscape).
//
// uso:
//   node browser/yt_cookies.mjs --json '{"mode":"login","timeoutMin":15}'   abre o Edge; voce faz login; exporta
//   node browser/yt_cookies.mjs --json '{"mode":"export"}'                   reexporta da sessao salva (janela fora da tela)
//
// O YouTube gira os cookies de sessao com o uso; o tkd reexporta quando o arquivo fica velho (footage.rs).
import fs from "node:fs";
import path from "node:path";
import { open, emit, log, evt, sleep, args, isLoggedInYT, profileDir } from "./lib.mjs";

const PROFILE = "_youtube";
const { mode = "export", timeoutMin = 15 } = args();
const out = path.join(profileDir(PROFILE), "cookies.txt");

/** Cookies do contexto no formato que o yt-dlp le (--cookies). */
function netscape(cookies) {
  const lines = ["# Netscape HTTP Cookie File", "# gerado por browser/yt_cookies.mjs (tkd); nao versionar", ""];
  for (const c of cookies) {
    if (!/(^|\.)(youtube|google)\.com$/.test(c.domain.replace(/^\./, ""))) continue;
    const domain = c.domain;
    const sub = domain.startsWith(".") ? "TRUE" : "FALSE";
    const exp = c.expires && c.expires > 0 ? Math.floor(c.expires) : 0;
    lines.push([domain, sub, c.path || "/", c.secure ? "TRUE" : "FALSE", exp, c.name, c.value].join("\t"));
  }
  return lines.join("\n") + "\n";
}

const { ctx, page } = await open({ profile: PROFILE, hidden: mode !== "login" });
try {
  if (mode === "login" && !(await isLoggedInYT(ctx))) {
    evt({ action: "Faça login com a conta Google descartável na janela do Edge" });
    await page.goto(
      "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F",
      { waitUntil: "domcontentloaded" }
    );
    const limit = Date.now() + timeoutMin * 60_000;
    while (!(await isLoggedInYT(ctx))) {
      if (page.isClosed()) throw new Error("janela fechada antes do login");
      if (Date.now() > limit) throw new Error(`tempo esgotado (${timeoutMin} min) esperando o login`);
      await sleep(2000);
    }
    log("login detectado");
  }
  // uma visita ao YouTube atualiza os cookies de sessao antes de exportar
  await page.goto("https://www.youtube.com/", { waitUntil: "domcontentloaded" });
  await sleep(3000);
  const logged = await isLoggedInYT(ctx);
  if (!logged) {
    emit({ ok: false, logged: false, error: "perfil _youtube sem login: rode `tkd yt-login`" });
  } else {
    const cookies = await ctx.cookies();
    fs.writeFileSync(out, netscape(cookies));
    const n = fs.readFileSync(out, "utf8").split("\n").filter((l) => l && !l.startsWith("#")).length;
    log(`${n} cookies exportados para ${out}`);
    emit({ ok: true, logged: true, file: out, n });
  }
} catch (e) {
  emit({ ok: false, error: String(e) });
} finally {
  await ctx.close().catch(() => {});
}
