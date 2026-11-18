// Linha de uma plataforma (TikTok / YouTube) com o login acontecendo ao vivo.
import { useEffect, useState, useTransition } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg } from "../lib/api";
import { keys } from "../lib/queries";
import { useLive } from "../lib/live";
import { ICheck, ISpin, ITikTok, IYouTube } from "./icons";
import { toast } from "./toast";
import { t } from "../lib/i18n";

const TIMEOUT_S = 600;

function Clock({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const s = Math.min(TIMEOUT_S, Math.floor((now - since) / 1000));
  const f = (n: number) => `${Math.floor(n / 60)}:${String(n % 60).padStart(2, "0")}`;
  return <span className="mono text-xs text-muted">{t("{a} de {b}", { a: f(s), b: f(TIMEOUT_S) })}</span>;
}

function Check({ state, children }: { state: "done" | "run" | "todo"; children: string }) {
  return (
    <div className="grid grid-cols-[20px_minmax(0,1fr)] items-center gap-2.5 py-2 text-[13px]">
      {state === "done" ? (
        <ICheck className="text-ok" />
      ) : state === "run" ? (
        <ISpin className="text-accent" />
      ) : (
        <span className="size-2 justify-self-center rounded-full shadow-[inset_0_0_0_1.5px_#5a5d66]" />
      )}
      <span className={state === "todo" ? "text-dim" : state === "done" ? "text-fg-2" : ""}>{children}</span>
    </div>
  );
}

export function PlatformLogin({ slug, platform, handle, compact = false }: {
  slug: string;
  platform: "tiktok" | "youtube";
  handle: string;
  compact?: boolean;
}) {
  const qc = useQueryClient();
  const [since, setSince] = useState<number | null>(null);
  const [checking, startCheck] = useTransition();
  const name = platform === "tiktok" ? "TikTok" : "YouTube";
  // o que o navegador esta fazendo neste login (vem dos eventos ao vivo)
  const login = useLive((s) => s.agents[slug]?.steps.findLast((st) => st.key === "login"));
  const action = login?.children.findLast((c) => c.kind === "browser")?.action ?? "";
  // login ja em andamento (voltou pra pagina do canal no meio): vem dos eventos ao vivo
  const liveWaiting = login?.status === "run" && login.action.includes(name);
  const started = since ?? (liveWaiting && login ? Date.parse(login.startedAt) : null);
  const waiting = started !== null;
  const found = action.startsWith("Achando") || action.startsWith("Lendo");

  const connect = async () => {
    setSince(Date.now());
    try {
      const h = await commands.login(slug, platform);
      toast(h ? t("{name} conectado: @{h}", { name, h }) : t("{name}: sessão salva", { name }));
    } catch (e) {
      const m = errMsg(e);
      if (!m.includes("cancelado")) toast(m, "error");
    } finally {
      setSince(null);
      await qc.invalidateQueries({ queryKey: keys.channel(slug) });
      await qc.invalidateQueries({ queryKey: keys.channels });
      await qc.invalidateQueries({ queryKey: keys.avatar(slug) });
    }
  };

  // sair em dois cliques: o 1o arma, o 2o confirma (volta sozinho depois de 4 s)
  const [armed, setArmed] = useState(false);
  const [leaving, startLeave] = useTransition();
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), 4000);
    return () => clearTimeout(t);
  }, [armed]);
  const logout = () => {
    if (!armed) return setArmed(true);
    setArmed(false);
    startLeave(async () => {
      try {
        await commands.logout(slug, platform);
        toast(t("{name}: desconectado de @{h}", { name, h: handle }));
      } catch (e) {
        toast(errMsg(e), "error");
      } finally {
        await qc.invalidateQueries({ queryKey: keys.channel(slug) });
        await qc.invalidateQueries({ queryKey: keys.channels });
        await qc.invalidateQueries({ queryKey: keys.avatar(slug) });
      }
    });
  };

  const test = () =>
    startCheck(async () => {
      try {
        const r = await commands.checkSession(slug, platform);
        toast(r.logged_in ? t("Sessão ativa como @{h}", { h: r.handle }) : t("{name}: sessão expirada, conecte de novo", { name }), r.logged_in ? "ok" : "error");
        await qc.invalidateQueries({ queryKey: keys.channel(slug) });
      } catch (e) {
        toast(errMsg(e), "error");
      }
    });

  return (
    <div className="border-t border-line-soft">
      <div className={`grid grid-cols-[44px_minmax(0,1fr)_auto] items-center gap-3.5 ${compact ? "px-5 py-3.5" : "px-5 py-4"}`}>
        <span
          className={`grid size-11 place-items-center rounded-xl ${platform === "youtube" ? "bg-accent-bg text-accent-ink" : "bg-[#24262b] text-fg"}`}
        >
          {platform === "tiktok" ? <ITikTok size={22} /> : <IYouTube size={22} />}
        </span>
        <div className="flex min-w-0 flex-col">
          <span className="font-semibold">{name}</span>
          {handle ? (
            <span className="mono truncate text-[13px] text-fg-2"><bdi>@{handle}</bdi></span>
          ) : (
            <span className="text-[13px] text-muted">
              {platform === "youtube" ? t("Shorts, entrando com a conta Google") : t("ainda não conectado")}
            </span>
          )}
        </div>
        <div className="flex items-center gap-3">
          {waiting ? (
            <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-accent">
              <ISpin size={14} />
              {t("Esperando login")}
            </span>
          ) : handle ? (
            <>
              <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-ok">
                <span className="size-[7px] rounded-full bg-ok" />
                {t("conectado")}
              </span>
              <button className="btn btn-sec btn-sm" type="button" onClick={test} disabled={checking}>
                {checking ? t("Conferindo…") : t("Testar login")}
              </button>
              <button className="btn btn-ghost btn-sm" type="button" onClick={connect}>
                {t("Trocar conta")}
              </button>
              <button
                className={`btn btn-sm ${armed ? "btn-danger" : "btn-ghost"}`}
                type="button"
                onClick={logout}
                disabled={leaving}
                title={t("Apaga só a sessão do {name} deste canal", { name })}
              >
                {leaving ? t("Saindo…") : armed ? t("Confirmar saída") : t("Sair")}
              </button>
            </>
          ) : (
            <button className="btn btn-pri btn-sm" type="button" onClick={connect}>
              {t("Conectar {name}", { name })}
            </button>
          )}
        </div>
      </div>
      {waiting && (
        <div aria-live="polite" className="anim-rise mx-5 mb-[18px] overflow-hidden rounded-xl bg-raise shadow-[0_0_0_1px_#2e3036]">
          <div className="relative h-0.5 overflow-hidden bg-[#1f2125]">
            <span className="anim-scan absolute inset-y-0 w-2/5 bg-[linear-gradient(90deg,transparent,#8b8cff,transparent)]" />
          </div>
          <div className="flex flex-col gap-2.5 px-[18px] py-4">
            <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1.5">
              <span className="flex-[1_1_260px] font-semibold">
                {platform === "youtube" ? t("Uma janela do Edge abriu. Entre na conta Google do canal.") : t("Uma janela do Edge abriu. Entre na conta do TikTok do canal.")}
              </span>
              <Clock since={started ?? Date.now()} />
            </div>
            <div>
              <Check state={action ? "done" : "run"}>{t("Janela aberta")}</Check>
              <Check state={found ? "done" : action ? "run" : "todo"}>{t("Esperando você entrar")}</Check>
              <Check state={found ? "run" : "todo"}>
                {platform === "youtube" ? t("Achar o canal do YouTube e guardar") : t("Guardar o @ da conta e a foto")}
              </Check>
            </div>
            <div className="flex flex-wrap gap-2 pt-0.5">
              <span className="text-xs text-dim">{t("O tkd percebe sozinho quando você entrar.")}</span>
              <button
                className="btn btn-ghost btn-sm ms-auto"
                type="button"
                onClick={() => void commands.cancelLogin(slug, platform)}
              >
                {t("Cancelar")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
