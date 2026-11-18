import { Link } from "@tanstack/react-router";
import type { ChannelSummary } from "../lib/api";
import { useAppStatus, useChannels, useSetup } from "../lib/queries";
import { activeStep, useLive } from "../lib/live";
import { Avatar, Dot, Page, PageHeader, Strip } from "../components/ui";
import { IAgents, IChevron, IPlus } from "../components/icons";
import { effortLabel } from "../lib/format";
import { t, tx } from "../lib/i18n";

const ROW =
  "grid grid-cols-[minmax(250px,1.5fr)_minmax(190px,1fr)_minmax(200px,1fr)_minmax(150px,.8fr)_16px] items-center gap-7 px-[22px]";

const TONE_TEXT: Record<string, string> = {
  ok: "text-ok",
  warn: "text-warn",
  error: "text-live",
  off: "text-fg-2",
  idle: "text-fg",
  run: "text-fg",
};

function Account({ label, handle }: { label: string; handle: string }) {
  return (
    <span className="inline-flex items-center gap-[5px] text-xs text-muted">
      <Dot tone={handle ? "ok" : "off"} className="!size-1.5" />
      {label}
    </span>
  );
}

/** "Agora": o rotulo do backend, trocado pelo que o agente esta fazendo neste segundo. */
function NowCell({ c }: { c: ChannelSummary }) {
  const step = useLive((s) => activeStep(s.agents[c.slug]));
  let label = tx(c.now.label);
  if (step) {
    label = tx(step.label);
    if (step.progress != null && step.key === "render") label = `${tx(step.label)} ${Math.round(step.progress)}%`;
  }
  return (
    <span className="flex min-w-0 flex-col gap-1">
      <span className={`truncate text-sm font-medium ${step ? "text-fg" : TONE_TEXT[c.now.tone] ?? ""}`}>{label}</span>
      <Strip stages={c.now.stages} />
    </span>
  );
}

function Row({ c }: { c: ChannelSummary }) {
  const ring = c.busy ? "#8b8cff" : c.now.tone === "ok" ? "#3ecf8e" : c.is_test || c.paused ? "#3a3d44" : "#5a5d66";
  return (
    <Link
      to="/canal/$slug"
      params={{ slug: c.slug }}
      className={`${ROW} group border-t border-line-soft py-4 !text-fg transition-colors duration-200 hover:bg-raise`}
    >
      <span className="flex min-w-0 items-center gap-3.5">
        <Avatar slug={c.slug} color={c.color} hasPhoto={c.has_avatar} size={40} ring={ring} />
        <span className="flex min-w-0 flex-col gap-1">
          <span className={`flex items-center gap-2 text-[15px] font-semibold tracking-[-0.01em] ${c.is_test ? "text-fg-2" : ""}`}>
            {c.slug}
            {c.own_model && <span className="rounded-[5px] bg-accent-soft px-[7px] py-px text-[11px] font-medium text-accent-ink">{t("modelo próprio")}</span>}
            {c.is_test && <span className="rounded-[5px] bg-[#1f2125] px-[7px] py-px text-[11px] font-medium text-fg-2">{t("teste")}</span>}
            {c.paused && <span className="rounded-[5px] bg-[#1f2125] px-[7px] py-px text-[11px] font-medium text-warn">{t("pausado")}</span>}
          </span>
          <span className="truncate text-[13px] text-muted">{c.is_test ? t("Só roda quando você manda") : c.niche || t("sem nicho definido")}</span>
        </span>
      </span>
      <span className="flex min-w-0 flex-col gap-1">
        {c.tiktok ? (
          <span className="mono truncate text-[13px]"><bdi>@{c.tiktok}</bdi></span>
        ) : (
          <span className="text-[13px] text-muted">{t("Sem conta ainda")}</span>
        )}
        <span className="flex gap-3">
          <Account label="TikTok" handle={c.tiktok} />
          <Account label="YouTube" handle={c.youtube} />
        </span>
      </span>
      <NowCell c={c} />
      <span className="flex min-w-0 flex-col gap-1">
        <span className={`text-sm font-medium whitespace-nowrap ${c.next_post ? "" : "text-muted"}`}>{tx(c.next_post_label)}</span>
        <span className="truncate text-[13px] text-muted">{tx(c.schedule_label)}</span>
      </span>
      <IChevron className="text-faint transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-fg rtl:group-hover:-translate-x-0.5" />
    </Link>
  );
}

function LiveBanner() {
  const order = useLive((s) => s.order);
  const agents = useLive((s) => s.agents);
  const running = order
    .map((c) => ({ c, st: activeStep(agents[c]) }))
    .filter((x) => x.st)
    .slice(0, 4);
  if (!running.length) return null;
  return (
    <Link to="/ao-vivo" className="panel anim-rise flex flex-wrap items-center gap-x-5 gap-y-3.5 px-5 py-4 !text-fg">
      <span className="inline-flex items-center gap-2 text-xs font-semibold text-live">
        <span className="anim-blink size-2 rounded-full bg-live" />
        {t("AO VIVO")}
      </span>
      <span className="flex flex-[1_1_360px] flex-wrap gap-x-[18px] gap-y-1.5 text-[13px] text-fg-2">
        {running.map(({ c, st }) => {
          const kid = st!.children.at(-1);
          const what = tx(kid?.status === "run" && kid.action ? kid.action : st!.action || st!.label);
          return (
            <span key={c}>
              <b className="font-semibold text-fg">{c}</b> {what.charAt(0).toLowerCase() + what.slice(1)}
              {st!.progress != null && st!.key === "render" ? ` ${Math.round(st!.progress)}%` : ""}
            </span>
          );
        })}
      </span>
      <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-accent">
        {t("Ver agentes trabalhando")}
        <IChevron size={14} />
      </span>
    </Link>
  );
}

/** Falta ferramenta obrigatoria: aponta para Primeiros passos antes de o usuario descobrir por um erro. */
function SetupBanner() {
  const { data } = useSetup();
  const missing = data?.filter((i) => i.status === "missing") ?? [];
  if (!missing.length) return null;
  return (
    <Link to="/primeiros-passos" className="panel flex flex-wrap items-center gap-3 px-5 py-4 hover:bg-hover">
      <span className="font-medium text-live">{missing.length === 1 ? t("Falta instalar 1 item") : t("Falta instalar {n} itens", { n: missing.length })}</span>
      <span className="min-w-0 flex-1 truncate text-muted">{missing.map((i) => tx(i.label)).join(", ")}</span>
      <span className="text-accent-ink">{t("Primeiros passos")} <span className="inline-block rtl:-scale-x-100">→</span></span>
    </Link>
  );
}

export default function Channels() {
  const { data: channels } = useChannels();
  const { data: st } = useAppStatus();
  return (
    <Page>
      <PageHeader title={t("Canais")} sub={t("Cada canal tem um agente próprio, uma conta e os canais que ele estuda.")}>
        <Link to="/agentes" className="btn btn-sec">
          <IAgents />
          {st ? st.model_label : t("Agentes")}
          {st && <span className="text-dim">· {t("esforço {e}", { e: effortLabel(st.effort) })}</span>}
        </Link>
        <Link to="/novo" className="btn btn-pri">
          <IPlus />
          {t("Novo canal")}
        </Link>
      </PageHeader>

      <SetupBanner />

      <section aria-labelledby="t-canais" className="panel">
        <h2 id="t-canais" className="sr-only">
          {t("Lista de canais")}
        </h2>
        <div className="overflow-x-auto">
          <div className="min-w-[900px]">
            <div className={`${ROW} pt-3.5 pb-2.5 text-xs font-medium text-dim`}>
              <span>{t("Canal")}</span>
              <span>{t("Conta")}</span>
              <span>{t("Agora")}</span>
              <span>{t("Próximo post")}</span>
              <span />
            </div>
            {channels.map((c) => (
              <Row key={c.slug} c={c} />
            ))}
            {!channels.length && (
              <div className="border-t border-line-soft px-[22px] py-10 text-center text-muted">
                {t("Nenhum canal ainda.")} <Link to="/novo">{t("Criar o primeiro")}</Link>
              </div>
            )}
          </div>
        </div>
        <div className="flex flex-wrap gap-x-[18px] gap-y-2 border-t border-line-soft px-[22px] py-3 text-xs text-dim">
          <span>{t("A barrinha mostra o dia do agente: buscar referências, estudar, criar a ideia, montar, avaliar e postar.")}</span>
        </div>
      </section>

      <LiveBanner />
    </Page>
  );
}
