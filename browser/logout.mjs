// Sai de UMA plataforma no perfil do canal, sem mexer na outra: o perfil .profiles/<canal>/ guarda TikTok e Google
// juntos, entao "sair do YouTube" apaga so os cookies do Google/YouTube (e vice-versa).
// uso: node browser/logout.mjs --json '{"profile":"meu-canal","platform":"youtube"}'
// saida: { ok, platform, removed, stillLoggedIn }
import { open, emit, log, args, isLoggedIn, isLoggedInYT } from "./lib.mjs";

const { profile, platform } = args();
const DOMAINS = {
  youtube: /(^|\.)(google\.[a-z.]+|youtube\.com|youtube-nocookie\.com|googlevideo\.com|gstatic\.com|googleusercontent\.com)$/,
  tiktok: /(^|\.)(tiktok\.com|tiktokv\.com|tiktokcdn\.com|byteoversea\.com|ibytedtos\.com)$/,
};

if (!profile || !DOMAINS[platform]) {
  emit({ ok: false, error: "profile e platform (tiktok|youtube) sao obrigatorios" });
  process.exit(0);
}

const { ctx } = await open({ profile, hidden: true });
try {
  const domain = DOMAINS[platform];
  const all = await ctx.cookies();
  const removed = all.filter((c) => domain.test(c.domain.replace(/^\./, ""))).length;
  await ctx.clearCookies({ domain });
  const stillLoggedIn = platform === "youtube" ? await isLoggedInYT(ctx) : await isLoggedIn(ctx);
  log(`${profile}: ${removed} cookies de ${platform} apagados`);
  emit({ ok: !stillLoggedIn, platform, removed, stillLoggedIn, ...(stillLoggedIn ? { error: "a sessao continuou ativa" } : {}) });
} catch (e) {
  emit({ ok: false, error: String(e) });
} finally {
  await ctx.close().catch(() => {});
}
