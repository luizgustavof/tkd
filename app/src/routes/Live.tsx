import { memo, useDeferredValue, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useChannelsLoose, useBrowserStatus } from "../lib/queries";
import { activeStep, useLive, type AgentNode, type ChildNode, type LogLine, type NodeStatus, type StepNode } from "../lib/live";
import { ago, duration, hms } from "../lib/format";
import { Avatar, Page } from "../components/ui";
import { ICheck, IGlobe, IX } from "../components/icons";

const TOOL_LABEL: Record<string, string> = {
  claude: "Claude",
  edge: "Edge",
  remotion: "Remotion",
  ffmpeg: "ffmpeg",
  python: "Python",
};

function StatusIcon({ s }: { s: NodeStatus }) {
  return (
    <span className="grid size-[18px] place-items-center" aria-hidden>
      {s === "done" ? (
        <ICheck size={14} className="text-ok" />
      ) : s === "run" ? (
        <span className="anim-pulse size-[9px] rounded-full bg-accent" />
      ) : s === "error" ? (
        <IX size={13} className="text-live" />
      ) : (
        <span className="size-[9px] rounded-full shadow-[inset_0_0_0_1.5px_#4a4d55]" />
      )}
    </span>
  );
}

function Tool({ tool, on }: { tool: string; on: boolean }) {
  if (!tool) return null;
  return (
    <span className={`inline-flex h-[22px] items-center rounded-md px-2 text-[11px] font-medium whitespace-nowrap ${on ? "bg-accent-soft text-accent-ink" : "bg-[#1f2125] text-fg-2"}`}>
      {TOOL_LABEL[tool] ?? tool}
    </span>
  );
}

type Sel = { channel: string; step?: string; child?: string };
type Flat = { id: string; level: 1 | 2 | 3; channel: string; parent?: string; sel: Sel; expandable: boolean };

const nodeCls = (selected: boolean) =>
  `grid w-full items-center gap-2.5 rounded-lg border-0 bg-transparent px-2.5 py-[7px] text-left transition-colors duration-150 ${
    selected ? "bg-accent-bg shadow-[inset_0_0_0_1px_#3d3f7a]" : "hover:bg-raise"
  }`;

/** Um agente (canal) na arvore. memo: so re-renderiza quando o no do canal muda. */
const AgentItem = memo(function AgentItem({ agent, color, hasPhoto, open, selected, focusId, onSelect, onToggle }: {
  agent: AgentNode;
  color: string;
  hasPhoto: boolean;
  open: boolean;
  selected: string;
  focusId: string;
  onSelect: (s: Sel, id: string) => void;
  onToggle: (ch: string) => void;
}) {
  const st = activeStep(agent);
  const id = `a:${agent.channel}`;
  const summary = st ? st.children.findLast((c) => c.status === "run")?.action || st.action || st.label : `parado · ${ago(agent.updatedAt)}`;
  return (
    <div role="none" className="border-t border-line-soft py-1.5 first:border-t-0">
      <button
        id={id}
        role="treeitem"
        aria-level={1}
        aria-expanded={open}
        aria-selected={selected === id}
        tabIndex={focusId === id ? 0 : -1}
        type="button"
        className={`${nodeCls(selected === id)} grid-cols-[30px_minmax(0,1fr)_auto] p-2.5`}
        onClick={() => {
          onSelect({ channel: agent.channel }, id);
          onToggle(agent.channel);
        }}
      >
        <Avatar slug={agent.channel} color={color} hasPhoto={hasPhoto} size={30} />
        <span className="flex min-w-0 flex-col">
          <span className="truncate font-medium">{agent.channel}</span>
          <span className="truncate text-xs text-dim">{summary}</span>
        </span>
        <span className={`text-xs font-medium ${st ? "text-accent" : "text-dim"}`}>{st ? "trabalhando" : "parado"}</span>
      </button>
      {open && (
        <div role="group" className="ml-[19px] border-l border-[#2a2c32] pl-3">
          {agent.steps.map((s) => (
            <StepItem key={s.id} step={s} channel={agent.channel} selected={selected} focusId={focusId} onSelect={onSelect} />
          ))}
          {!agent.steps.length && <span className="block px-2.5 py-2 text-xs text-dim">Nada ainda.</span>}
        </div>
      )}
    </div>
  );
});

function StepItem({ step, channel, selected, focusId, onSelect }: {
  step: StepNode;
  channel: string;
  selected: string;
  focusId: string;
  onSelect: (s: Sel, id: string) => void;
}) {
  const id = `s:${step.id}`;
  return (
    <div role="none">
      <button
        id={id}
        role="treeitem"
        aria-level={2}
        aria-selected={selected === id}
        tabIndex={focusId === id ? 0 : -1}
        type="button"
        className={`${nodeCls(selected === id)} grid-cols-[18px_minmax(0,1fr)_auto]`}
        onClick={() => onSelect({ channel, step: step.id }, id)}
      >
        <StatusIcon s={step.status} />
        <span className="flex min-w-0 flex-col">
          <span className={`truncate text-sm ${step.status === "run" ? "font-medium text-fg" : step.status === "done" ? "text-muted" : "text-fg-2"}`}>{step.label}</span>
          <span className="truncate text-xs text-dim">{step.status === "error" ? step.detail : step.action || step.detail}</span>
          {step.status === "run" && step.progress != null && (
            <span className="mt-1 block h-[3px] overflow-hidden rounded-sm bg-line">
              <span className="block h-full bg-accent transition-[width] duration-300" style={{ width: `${step.progress}%` }} />
            </span>
          )}
        </span>
        <span className="mono text-[11px] whitespace-nowrap text-[#6f727d]">{hms(step.startedAt).slice(0, 5)}</span>
      </button>
      {step.children.length > 0 && (
        <div role="group" className="ml-[19px] border-l border-[#24252a] pl-3">
          {step.children.slice(-6).map((c) => (
            <ChildItem key={c.id} c={c} channel={channel} step={step.id} selected={selected} focusId={focusId} onSelect={onSelect} />
          ))}
        </div>
      )}
    </div>
  );
}

function ChildItem({ c, channel, step, selected, focusId, onSelect }: {
  c: ChildNode;
  channel: string;
  step: string;
  selected: string;
  focusId: string;
  onSelect: (s: Sel, id: string) => void;
}) {
  const id = `c:${c.id}`;
  return (
    <button
      id={id}
      role="treeitem"
      aria-level={3}
      aria-selected={selected === id}
      tabIndex={focusId === id ? 0 : -1}
      type="button"
      className={`${nodeCls(selected === id)} grid-cols-[18px_minmax(0,1fr)_auto]`}
      onClick={() => onSelect({ channel, step, child: c.id }, id)}
    >
      <StatusIcon s={c.status} />
      <span className="flex min-w-0 flex-col">
        <span className={`truncate text-[13px] ${c.status === "run" ? "text-fg" : "text-muted"}`}>{c.action || c.label}</span>
        {c.status === "run" && c.progress != null && (
          <span className="mt-1 block h-[3px] overflow-hidden rounded-sm bg-line">
            <span className="block h-full bg-accent transition-[width] duration-300" style={{ width: `${c.progress}%` }} />
          </span>
        )}
      </span>
      <Tool tool={c.tool} on={c.status === "run"} />
    </button>
  );
}

function flatten(order: string[], agents: Record<string, AgentNode>, open: Set<string>): Flat[] {
  const out: Flat[] = [];
  for (const ch of order) {
    const a = agents[ch];
    if (!a) continue;
    const aid = `a:${ch}`;
    out.push({ id: aid, level: 1, channel: ch, sel: { channel: ch }, expandable: true });
    if (!open.has(ch)) continue;
    for (const s of a.steps) {
      const sid = `s:${s.id}`;
      out.push({ id: sid, level: 2, channel: ch, parent: aid, sel: { channel: ch, step: s.id }, expandable: false });
      for (const c of s.children.slice(-6)) out.push({ id: `c:${c.id}`, level: 3, channel: ch, parent: sid, sel: { channel: ch, step: s.id, child: c.id }, expandable: false });
    }
  }
  return out;
}

function LogList({ log }: { log: LogLine[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: log.length, getScrollElement: () => ref.current, estimateSize: () => 26, overscan: 12 });
  // segue o fim enquanto chegam linhas novas
  useEffect(() => {
    if (log.length) v.scrollToIndex(log.length - 1, { align: "end" });
  }, [log.length, v]);
  if (!log.length) return <span className="text-[13px] text-dim">Sem registro ainda.</span>;
  return (
    <div ref={ref} className="max-h-[300px] overflow-y-auto" role="log" aria-label="Registro">
      <div className="relative w-full" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((it) => {
          const l = log[it.index]!;
          return (
            <div
              key={it.key}
              ref={v.measureElement}
              data-index={it.index}
              className="absolute top-0 left-0 grid w-full grid-cols-[62px_minmax(0,1fr)] gap-2.5 py-[5px] text-[12.5px]"
              style={{ transform: `translateY(${it.start}px)` }}
            >
              <span className="mono pt-0.5 text-[11px] text-[#6f727d]">{hms(l.at)}</span>
              <span className={`break-words ${l.tone === "error" ? "text-live" : l.tone === "ok" ? "text-ok" : "text-fg-2"}`}>{l.text}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Inspector({ sel, color, hasPhoto }: { sel: Sel; color: string; hasPhoto: boolean }) {
  const agent = useLive((s) => s.agents[sel.channel]);
  const [now, setNow] = useState(() => Date.now());
  // relogio so anda com etapa rodando (parado nao ha tempo correndo para mostrar)
  const running = !!agent?.steps.some((s) => s.status === "run");
  useEffect(() => {
    if (!running) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);
  if (!agent) return null;
  const step = sel.step ? agent.steps.find((s) => s.id === sel.step) : activeStep(agent) ?? agent.steps.at(-1);
  const child = sel.child ? step?.children.find((c) => c.id === sel.child) : step?.children.findLast((c) => c.status === "run");
  const node = child ?? step;
  const status = node?.status ?? "wait";
  const title = node ? (child ? child.action || child.label : step!.label) : "Parado";
  const tool = node?.tool ?? "";
  const model = [...(step?.children ?? [])].reverse().find((c) => c.kind === "agent")?.action.match(/\(([^)]+)\)/)?.[1];
  const url = [...(step?.log ?? [])].reverse().find((l) => l.text.startsWith("url "))?.text.slice(4);
  const progress = node?.progress ?? null;
  const elapsed = node ? (node.endedAt ? new Date(node.endedAt).getTime() : now) - new Date(node.startedAt).getTime() : 0;
  const nextIdx = step ? ["scout", "study", "ideate", "render", "review", "publish"].indexOf(step.key) : -1;
  const NEXT = ["Estudar", "Criar a ideia", "Montar o vídeo", "Avaliar", "Postar", "Esperar o próximo horário"];

  return (
    <section className="panel flex min-w-0 flex-col gap-4 p-5" aria-labelledby="t-insp">
      <div className="flex items-center gap-3">
        <Avatar slug={agent.channel} color={color} hasPhoto={hasPhoto} size={30} />
        <div className="flex min-w-0 flex-1 flex-col">
          <h2 id="t-insp" className="m-0 truncate text-base font-semibold">
            {title}
          </h2>
          <span className="text-[13px] text-muted">
            {agent.channel} · {step?.label ?? "sem etapa"}
          </span>
        </div>
        <span className={`text-xs font-medium ${status === "run" ? "text-accent" : status === "done" ? "text-ok" : status === "error" ? "text-live" : "text-dim"}`}>
          {status === "run" ? "rodando" : status === "done" ? "pronto" : status === "error" ? "erro" : "parado"}
        </span>
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-xs font-medium text-dim">O que está vendo</span>
        <div
          className="relative mx-auto flex aspect-[9/16] max-h-[360px] w-full max-w-[203px] flex-col justify-end overflow-hidden rounded-xl bg-[#08090a] shadow-[0_0_0_1px_var(--color-line)]"
          role="img"
          aria-label={node?.action || title}
        >
          <span className="pointer-events-none absolute inset-0 bg-[repeating-linear-gradient(0deg,rgba(255,255,255,.025)_0_1px,transparent_1px_3px)]" />
          <div className="absolute top-2.5 right-2.5 left-2.5 flex items-center justify-between">
            <Tool tool={tool} on={status === "run"} />
            {status === "run" && (
              <span className="inline-flex items-center gap-1.5 text-[11px] font-semibold text-live">
                <span className="anim-blink size-1.5 rounded-full bg-live" />
                REC
              </span>
            )}
          </div>
          <div className="relative flex flex-1 items-center justify-center px-4 text-center text-[13px] text-fg-2">
            {tool === "edge" ? <IGlobe size={28} className="text-faint" /> : null}
            <span className="absolute inset-x-4 bottom-16">{node?.action || step?.action || "Aguardando"}</span>
          </div>
          <div className="relative flex flex-col gap-1.5 p-3">
            {url && <span className="mono truncate text-[10.5px] text-dim">{url.replace(/^https?:\/\//, "")}</span>}
            <span className="block h-[3px] overflow-hidden rounded-sm bg-line">
              <span className="block h-full bg-accent transition-[width] duration-300" style={{ width: `${progress ?? (status === "done" ? 100 : 0)}%` }} />
            </span>
            <span className="mono text-[11px] text-dim">{progress != null ? `${Math.round(progress)}%` : duration(elapsed)}</span>
          </div>
        </div>
      </div>

      <div>
        {[
          ["Ferramenta", TOOL_LABEL[tool] ?? (tool || "—")],
          ["Modelo", model ?? "—"],
          ["Começou", node ? `${hms(node.startedAt)} · ${duration(elapsed)}` : "—"],
          ["Depois disso", nextIdx >= 0 ? NEXT[nextIdx] : "—"],
        ].map(([k, v]) => (
          <div key={k} className="grid grid-cols-[110px_minmax(0,1fr)] gap-2.5 border-t border-[#1f2125] py-2 text-[13px]">
            <span className="text-dim">{k}</span>
            <span className={k === "Começou" ? "mono" : ""}>{v}</span>
          </div>
        ))}
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium text-dim">Pensando em voz alta</span>
        <LogList log={step?.log ?? []} />
      </div>
    </section>
  );
}

function BrowserLock() {
  const { data } = useBrowserStatus();
  const h = data?.holder;
  return (
    <div className="flex items-center gap-2.5 rounded-xl bg-raise px-4 py-3 text-[13px] shadow-[0_0_0_1px_var(--color-line)]">
      <IGlobe className="text-muted" />
      {h ? (
        <span>
          Navegador: <b className="font-semibold">{h.channel}</b> usando
        </span>
      ) : (
        <span className="text-muted">Navegador livre</span>
      )}
      {!!data?.waiting.length && <span className="text-muted">· {data.waiting.join(", ")} {data.waiting.length > 1 ? "esperam" : "espera"}</span>}
    </div>
  );
}

export default function Live() {
  const order = useLive((s) => s.order);
  const agents = useLive((s) => s.agents);
  const connected = useLive((s) => s.connected);
  const { data: channels } = useChannelsLoose();
  // null = ainda nao mexeu: abre os que estao rodando (ou o primeiro)
  const [open, setOpen] = useState<Set<string> | null>(null);
  const [sel, setSel] = useState<{ s: Sel; id: string } | null>(null);
  const [focusId, setFocusId] = useState("");
  const treeRef = useRef<HTMLDivElement>(null);

  // primeiro agente rodando fica aberto e selecionado
  const firstRunning = order.find((c) => activeStep(agents[c])) ?? order[0];
  const current = sel ?? (firstRunning ? { s: { channel: firstRunning }, id: `a:${firstRunning}` } : null);
  const deferred = useDeferredValue(current);
  const running = order.filter((c) => activeStep(agents[c]));
  const shown = open ?? new Set(running.length ? running : firstRunning ? [firstRunning] : []);
  const flat = flatten(order, agents, shown);
  const focus = focusId && flat.some((f) => f.id === focusId) ? focusId : (current?.id ?? flat[0]?.id ?? "");
  const meta = (slug: string) => channels?.find((c) => c.slug === slug);

  const toggle = (ch: string) =>
    setOpen(() => {
      const n = new Set(shown);
      if (n.has(ch)) n.delete(ch);
      else n.add(ch);
      return n;
    });
  const select = (s: Sel, id: string) => {
    setSel({ s, id });
    setFocusId(id);
  };
  const move = (id: string) => {
    setFocusId(id);
    document.getElementById(id)?.focus();
  };
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const i = flat.findIndex((f) => f.id === focus);
    const cur = flat[i];
    if (!cur) return;
    if (e.key === "ArrowDown" && flat[i + 1]) move(flat[i + 1]!.id);
    else if (e.key === "ArrowUp" && flat[i - 1]) move(flat[i - 1]!.id);
    else if (e.key === "Home" && flat[0]) move(flat[0].id);
    else if (e.key === "End" && flat.at(-1)) move(flat.at(-1)!.id);
    else if (e.key === "ArrowRight" && cur.level === 1 && !shown.has(cur.channel)) toggle(cur.channel);
    else if (e.key === "ArrowLeft") {
      if (cur.level === 1 && shown.has(cur.channel)) toggle(cur.channel);
      else if (cur.parent) move(cur.parent);
    } else if (e.key === "Enter" || e.key === " ") select(cur.sel, cur.id);
    else return;
    e.preventDefault();
  };

  return (
    <Page>
      <header className="flex flex-wrap items-end gap-4">
        <div className="flex flex-[1_1_360px] flex-col gap-1">
          <div className="flex items-center gap-3">
            <h1 className="m-0 text-2xl font-semibold tracking-[-0.02em]">Ao vivo</h1>
            <span className="inline-flex items-center gap-[7px] text-xs font-semibold tracking-[.02em] text-live">
              <span className={`size-2 rounded-full bg-live ${connected ? "anim-blink" : "opacity-40"}`} />
              {connected ? "AGORA" : "CONECTANDO"}
            </span>
          </div>
          <p className="m-0 text-muted">O que cada agente está fazendo, vendo e usando neste momento. Clique num passo pra ver de perto.</p>
        </div>
        <BrowserLock />
      </header>

      <div className="grid grid-cols-[minmax(0,1.2fr)_minmax(300px,1fr)] items-start gap-6 max-[1000px]:grid-cols-1">
        <section className="panel p-2" aria-labelledby="t-tree">
          <h2 id="t-tree" className="sr-only">
            Árvore dos agentes
          </h2>
          {order.length ? (
            <div ref={treeRef} role="tree" aria-labelledby="t-tree" onKeyDown={onKey}>
              {order.map((c) => {
                const a = agents[c];
                if (!a) return null;
                const m = meta(c);
                return (
                  <AgentItem
                    key={c}
                    agent={a}
                    color={m?.color ?? "#2b2d31"}
                    hasPhoto={m?.has_avatar ?? false}
                    open={shown.has(c)}
                    selected={current?.id ?? ""}
                    focusId={focus}
                    onSelect={select}
                    onToggle={toggle}
                  />
                );
              })}
            </div>
          ) : (
            <p className="m-0 px-3 py-10 text-center text-muted">
              Nenhum agente trabalhou ainda nesta semana. Faça um vídeo de teste num canal pra ver aqui.
            </p>
          )}
        </section>
        {deferred && <Inspector sel={deferred.s} color={meta(deferred.s.channel)?.color ?? "#2b2d31"} hasPhoto={meta(deferred.s.channel)?.has_avatar ?? false} />}
      </div>
    </Page>
  );
}
