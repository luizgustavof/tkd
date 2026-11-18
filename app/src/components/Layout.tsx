import { Suspense, useTransition } from "react";
import { Link, Outlet } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg } from "../lib/api";
import { keys, useAppStatus, useChannelsLoose, useSetup } from "../lib/queries";
import { useLive } from "../lib/live";
import { Avatar, Dot, Skeleton, Switch } from "./ui";
import { IAgents, IChannels, ILive, IUsage, IPlus, IShield } from "./icons";
import { Toaster, toast } from "./toast";

const navCls =
  "flex h-[34px] items-center gap-2.5 rounded-[7px] px-2.5 font-medium text-fg-2 transition-colors hover:bg-hover hover:text-fg";
const navOn = { className: "bg-hover !text-fg shadow-[0_1px_2px_rgba(0,0,0,.3),0_0_0_1px_var(--color-line)]" };

function Logo() {
  return (
    <div className="flex items-center gap-2.5 px-2.5">
      <svg width="22" height="22" viewBox="0 0 22 22" aria-hidden>
        <rect x="1" y="3" width="20" height="4" rx="1.5" fill="#ececef" />
        <rect x="1" y="9" width="13" height="4" rx="1.5" fill="#8b8cff" />
        <rect x="1" y="15" width="17" height="4" rx="1.5" fill="#5a5d66" />
      </svg>
      <span className="text-base font-semibold tracking-[-0.02em]">tkd</span>
    </div>
  );
}

function AnyRunning() {
  const running = useLive((s) => s.order.some((c) => s.agents[c]?.steps.some((st) => st.status === "run")));
  return running ? <span aria-label="agentes trabalhando" className="anim-blink ml-auto size-[7px] rounded-full bg-live" /> : null;
}

function Autopilot() {
  const { data } = useAppStatus();
  const qc = useQueryClient();
  const [pending, start] = useTransition();
  const on = data?.autopilot ?? false;
  const toggle = (v: boolean) =>
    start(async () => {
      try {
        await (v ? commands.watchStart() : commands.watchStop());
        toast(v ? "Piloto automático ligado" : "Piloto automático desligado");
      } catch (e) {
        toast(errMsg(e), "error");
      }
      await qc.invalidateQueries({ queryKey: keys.status });
    });
  return (
    <div className="mt-auto flex flex-col gap-1 border-t border-line px-2.5 pt-3">
      <label className="flex items-center gap-2 font-medium">
        <Dot tone={on ? "ok" : "off"} />
        <span className="flex-1">{on ? "Piloto automático ligado" : "Piloto automático desligado"}</span>
        <Switch checked={on} disabled={pending || !data} onChange={toggle} label="Piloto automático" />
      </label>
      <span className="text-xs text-muted">
        {on
          ? "Os agentes trabalham sozinhos e postam no horário de cada canal."
          : "Nada roda sozinho. Você ainda pode fazer vídeos de teste."}
      </span>
    </div>
  );
}

function ChannelNav() {
  const { data } = useChannelsLoose();
  if (!data)
    return (
      <div className="flex flex-col gap-2 px-2.5">
        <Skeleton className="h-5 w-24" />
        <Skeleton className="h-5 w-20" />
      </div>
    );
  return (
    <>
      {data.map((c) => (
        <Link key={c.slug} to="/canal/$slug" params={{ slug: c.slug }} className={navCls} activeProps={navOn}>
          <Avatar slug={c.slug} color={c.color} hasPhoto={c.has_avatar} />
          <span className="truncate">{c.slug}</span>
          {c.busy && <Dot tone="run" pulse className="ml-auto" />}
        </Link>
      ))}
    </>
  );
}

/** Ponto no menu quando falta algo obrigatorio (ou so aviso). */
function SetupDot() {
  const { data } = useSetup();
  const missing = data?.filter((i) => i.status === "missing").length ?? 0;
  const warn = data?.filter((i) => i.status === "warn").length ?? 0;
  if (!missing && !warn) return null;
  return (
    <span className={`mono ml-auto text-xs ${missing ? "text-live" : "text-warn"}`} title={missing ? `${missing} faltando` : `${warn} aviso(s)`}>
      {missing || warn}
    </span>
  );
}

function Count() {
  const { data } = useChannelsLoose();
  return <span className="mono ml-auto text-xs text-dim">{data?.length ?? ""}</span>;
}

export function Layout() {
  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 flex h-screen w-[248px] flex-none flex-col gap-[22px] overflow-y-auto border-r border-line px-3.5 py-5 max-[900px]:w-[208px]">
        <Logo />
        <nav aria-label="Principal" className="flex flex-col gap-0.5">
          <Link to="/" className={navCls} activeProps={navOn} activeOptions={{ exact: true }}>
            <IChannels />
            Canais
            <Count />
          </Link>
          <Link to="/ao-vivo" className={navCls} activeProps={navOn}>
            <ILive />
            Ao vivo
            <AnyRunning />
          </Link>
          <Link to="/agentes" className={navCls} activeProps={navOn}>
            <IAgents />
            Agentes
          </Link>
          <Link to="/cortes" className={navCls} activeProps={navOn}>
            <ILive />
            Cortes
          </Link>
          <Link to="/consumo" className={navCls} activeProps={navOn}>
            <IUsage />
            Consumo
          </Link>
          <Link to="/novo" className={navCls} activeProps={navOn}>
            <IPlus />
            Novo canal
          </Link>
          <Link to="/primeiros-passos" className={navCls} activeProps={navOn}>
            <IShield />
            Primeiros passos
            <SetupDot />
          </Link>
        </nav>
        <div className="flex flex-col gap-0.5">
          <span className="px-2.5 pb-1.5 text-xs text-dim">Canais</span>
          <ChannelNav />
        </div>
        <Autopilot />
      </aside>
      <main className="min-w-0 flex-1 px-10 pt-7 pb-12 max-[900px]:px-5">
        <Suspense fallback={<PageSkeleton />}>
          <Outlet />
        </Suspense>
      </main>
      <Toaster />
    </div>
  );
}

export function PageSkeleton() {
  return (
    <div className="flex max-w-[1120px] flex-col gap-7" aria-busy="true">
      <div className="flex flex-col gap-2">
        <Skeleton className="h-7 w-40" />
        <Skeleton className="h-4 w-96 max-w-full" />
      </div>
      <div className="panel flex flex-col gap-4 p-5">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="flex items-center gap-4">
            <Skeleton className="size-10 !rounded-full" />
            <div className="flex flex-1 flex-col gap-2">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-3 w-64 max-w-full" />
            </div>
            <Skeleton className="h-4 w-24" />
          </div>
        ))}
      </div>
    </div>
  );
}
