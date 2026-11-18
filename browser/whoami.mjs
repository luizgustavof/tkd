import { open, isLoggedIn, whoami, isLoggedInYT, whoamiYT, emit, args } from "./lib.mjs";

const { profile = "default", platform = "tiktok" } = args();
const { ctx, page, dir } = await open({ profile, hidden: true });
try {
  if (platform === "youtube") {
    const logged = await isLoggedInYT(ctx);
    const me = logged ? await whoamiYT(page) : { handle: "", channelId: "" };
    emit({ ok: true, profile, dir, platform, loggedIn: logged, handle: me.handle || me.channelId, channelId: me.channelId });
  } else {
    const logged = await isLoggedIn(ctx);
    const handle = logged ? await whoami(page) : "";
    emit({ ok: true, profile, dir, platform, loggedIn: logged, handle });
  }
} catch (e) {
  emit({ ok: false, profile, error: String(e) });
} finally {
  await ctx.close();
}
