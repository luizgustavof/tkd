// Estado ao vivo dos agentes: store externa alimentada pelo Channel do Tauri.
// Cada evento troca so o no do canal afetado, entao `useLive(sel)` re-renderiza
// apenas quem le aquele pedaco (useSyncExternalStore + seletor).
import { useSyncExternalStore } from "react";
import { Channel } from "@tauri-apps/api/core";
import { commands, type EventDto, type LiveBatch } from "./api";
import { t } from "./i18n";

export type NodeStatus = "run" | "done" | "error" | "wait";

export type LogLine = { at: string; text: string; tone?: "error" | "ok" };

export type ChildNode = {
  id: string;
  kind: string; // agent | browser
  label: string;
  action: string;
  tool: string;
  status: NodeStatus;
  progress: number | null;
  startedAt: string;
  endedAt: string | null;
  detail: string;
};

export type StepNode = {
  id: string;
  key: string;
  label: string;
  action: string;
  tool: string;
  status: NodeStatus;
  progress: number | null;
  startedAt: string;
  endedAt: string | null;
  detail: string;
  log: LogLine[];
  children: ChildNode[];
};

export type AgentNode = {
  channel: string;
  runId: string;
  steps: StepNode[];
  updatedAt: string;
};

export type LiveState = {
  agents: Record<string, AgentNode>;
  /** canais na ordem do ultimo movimento */
  order: string[];
  connected: boolean;
};

const SUB = new Set(["agent", "browser"]);
const MAX_LOG = 300;

let state: LiveState = { agents: {}, order: [], connected: false };
const listeners = new Set<() => void>();
const changeHooks = new Set<(e: EventDto) => void>();

function statusOf(s: string): NodeStatus {
  if (s === "ok") return "done";
  if (s === "error") return "error";
  return "run";
}

function pushLog(log: LogLine[], line: LogLine): LogLine[] {
  const next = log.length >= MAX_LOG ? log.slice(log.length - MAX_LOG + 1) : log.slice();
  next.push(line);
  return next;
}

function apply(agents: Record<string, AgentNode>, e: EventDto): Record<string, AgentNode> {
  const prev = agents[e.channel];
  let a: AgentNode = prev ?? { channel: e.channel, runId: e.run_id, steps: [], updatedAt: e.at };

  // video novo: a arvore recomeca
  if (!SUB.has(e.step) && e.status === "start" && e.run_id && e.run_id !== a.runId) {
    const allDone = a.steps.every((s) => s.status !== "run");
    if (allDone) a = { ...a, runId: e.run_id, steps: [] };
  }

  const steps = a.steps.slice();
  if (SUB.has(e.step)) {
    // agente / navegador entram como filhos da etapa que esta rodando
    let pi = -1;
    for (let i = steps.length - 1; i >= 0; i--) if (steps[i]!.status === "run") { pi = i; break; }
    if (pi === -1) {
      // fim de filho sem etapa aberta (ex.: run fechado como interrompido): nada a mostrar
      if (e.status === "ok" || e.status === "error") return agents;
      steps.push(newStep(e, e.step));
      pi = steps.length - 1;
    }
    const parent = { ...steps[pi]! };
    const kids = parent.children.slice();
    let ci = -1;
    for (let i = kids.length - 1; i >= 0; i--) if (kids[i]!.kind === e.step) { ci = i; break; }
    if (e.status === "start" || ci === -1 || kids[ci]!.status !== "run") {
      kids.push({
        id: `${parent.id}/${e.step}/${kids.length}`,
        kind: e.step,
        label: e.step_label,
        action: e.action,
        tool: e.tool,
        status: "run",
        progress: e.progress ?? null,
        startedAt: e.at,
        endedAt: null,
        detail: "",
      });
      ci = kids.length - 1;
    }
    const c = { ...kids[ci]! };
    if (e.action) c.action = e.action;
    if (e.tool) c.tool = e.tool;
    if (e.progress != null) c.progress = e.progress;
    if (e.status === "ok" || e.status === "error") {
      c.status = statusOf(e.status);
      c.endedAt = e.at;
      c.detail = e.detail;
    }
    kids[ci] = c;
    parent.children = kids;
    if (e.status === "info" && e.detail) parent.log = pushLog(parent.log, { at: e.at, text: e.detail });
    else if (e.action && e.action !== c.label && e.status === "progress" && e.progress == null)
      parent.log = pushLog(parent.log, { at: e.at, text: e.action });
    if (e.status === "error") parent.log = pushLog(parent.log, { at: e.at, text: e.detail, tone: "error" });
    // o progresso do navegador vale para a etapa (ex.: video 4 de 9)
    if (e.progress != null && e.step === "browser") parent.progress = e.progress;
    // etapa criada so para hospedar este filho (ex.: "Testar login" usa o navegador fora de etapa):
    // fecha junto com ele, senao o canal fica "usando o navegador" pra sempre
    if (parent.key === e.step && (e.status === "ok" || e.status === "error") && parent.children.every((k) => k.status !== "run")) {
      parent.status = statusOf(e.status);
      parent.endedAt = e.at;
    }
    steps[pi] = parent;
  } else {
    let si = -1;
    for (let i = steps.length - 1; i >= 0; i--) if (steps[i]!.key === e.step) { si = i; break; }
    if (si === -1 || (e.status === "start" && steps[si]!.status !== "run")) {
      if (si !== -1 && e.status === "start") steps.splice(si, 1);
      steps.push(newStep(e, e.step));
      si = steps.length - 1;
    }
    const s = { ...steps[si]! };
    if (e.action) s.action = e.action;
    if (e.tool) s.tool = e.tool;
    if (e.progress != null) s.progress = e.progress;
    if (e.status === "ok" || e.status === "error") {
      s.status = statusOf(e.status);
      s.endedAt = e.at;
      s.detail = e.detail;
      s.log = pushLog(s.log, { at: e.at, text: e.detail || (e.status === "ok" ? t("pronto") : t("erro")), tone: e.status === "ok" ? "ok" : "error" });
      // filhos abertos fecham junto
      s.children = s.children.map((c) => (c.status === "run" ? { ...c, status: s.status, endedAt: e.at } : c));
    } else if (e.status === "info" && e.detail) {
      s.log = pushLog(s.log, { at: e.at, text: e.detail });
    } else if (e.status === "progress" && e.action && e.progress == null) {
      s.log = pushLog(s.log, { at: e.at, text: e.action });
    }
    steps[si] = s;
  }
  return { ...agents, [e.channel]: { ...a, steps, updatedAt: e.at } };
}

function newStep(e: EventDto, key: string): StepNode {
  return {
    id: `${e.channel}/${key}/${e.at}`,
    key,
    label: e.step_label,
    action: e.action,
    tool: e.tool,
    status: "run",
    progress: e.progress ?? null,
    startedAt: e.at,
    endedAt: null,
    detail: "",
    log: [],
    children: [],
  };
}

function ingest(batch: LiveBatch) {
  let agents = batch.initial ? {} : state.agents;
  for (const e of batch.events) agents = apply(agents, e);
  const order = Object.values(agents)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .map((a) => a.channel);
  state = { agents, order, connected: true };
  for (const l of listeners) l();
  if (!batch.initial) for (const e of batch.events) for (const h of changeHooks) h(e);
}

let started = false;

/** Liga o fluxo ao vivo uma vez por janela. */
export function startLive() {
  if (started) return;
  started = true;
  const ch = new Channel<LiveBatch>();
  ch.onmessage = ingest;
  commands.liveSubscribe(ch as never).catch((e) => {
    started = false;
    console.error("ao vivo:", e);
  });
}

/** Chamado a cada evento novo (ex.: invalidar consultas quando uma etapa termina). */
export function onLiveEvent(fn: (e: EventDto) => void): () => void {
  changeHooks.add(fn);
  return () => changeHooks.delete(fn);
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function useLive<T>(sel: (s: LiveState) => T): T {
  return useSyncExternalStore(subscribe, () => sel(state));
}

/** Canal excluido: some do Ao vivo sem esperar recarregar. */
export function forgetLive(channel: string) {
  if (!state.agents[channel]) return;
  const { [channel]: _, ...agents } = state.agents;
  state = { ...state, agents, order: state.order.filter((c) => c !== channel) };
  for (const l of listeners) l();
}

export function getLive(): LiveState {
  return state;
}

/** Etapa rodando agora (a mais recente), se houver. */
export function activeStep(a: AgentNode | undefined): StepNode | undefined {
  if (!a) return undefined;
  for (let i = a.steps.length - 1; i >= 0; i--) if (a.steps[i]!.status === "run") return a.steps[i];
  return undefined;
}

export function isRunning(a: AgentNode | undefined): boolean {
  return !!activeStep(a);
}
