import { useState, useTransition } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg, type AgentConfig, type ChannelAgent, type TaskRow } from "../lib/api";
import { keys, useAgentConfig, useChannelsLoose } from "../lib/queries";
import { activeStep, useLive } from "../lib/live";
import { EFFORT_LEVEL, effortLabel, modelLabel } from "../lib/format";
import { Avatar, Dialog, EffortBars, Page, PageHeader, Panel } from "../components/ui";
import { ISpin } from "../components/icons";
import { toast } from "../components/toast";
import { Select } from "../components/Select";
import { t, tx } from "../lib/i18n";
import type { MsgKey } from "../lib/messages";

const MODEL_DESC: Record<string, MsgKey> = {
  "claude-opus-5-5": "O mais forte. Melhor pra estudar e criar.",
  "claude-sonnet-5-5": "Rápido e bom. Equilíbrio de custo.",
  "claude-haiku-5-5": "O mais leve. Bom pra tarefas simples.",
};
const TASK_DESC: Record<string, MsgKey> = {
  study: "Assiste as referências e escreve o guia de estilo",
  ideate: "Escreve o roteiro de cada vídeo",
  review: "Olha os frames e dá a nota",
  recreate: "Refaz um edit de referência quadro a quadro",
  footage: "Decide o que buscar de imagem",
  tag_clips: "Descreve os clipes baixados",
};

type Target = { kind: "global" } | { kind: "task"; task: TaskRow } | { kind: "channel"; ch: ChannelAgent };

function summaryOf(model: string, effort: string, cfg: AgentConfig) {
  return t("{m} · esforço {e}", { m: modelLabel(model || cfg.model), e: effortLabel(effort || cfg.effort) });
}

function ModelPicker({ value, onChange, models, allowInherit }: {
  value: string;
  onChange: (v: string) => void;
  models: { value: string; label: string }[];
  allowInherit: boolean;
}) {
  const opts = allowInherit ? [{ value: "", label: t("Padrão") }, ...models] : models;
  return (
    <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label={t("Modelo")}>
      {opts.map((m) => {
        const on = m.value === value;
        return (
          <button
            key={m.value || "inherit"}
            type="button"
            role="radio"
            aria-checked={on}
            onClick={() => onChange(m.value)}
            className={`grid grid-cols-[minmax(0,1fr)_18px] gap-x-2.5 gap-y-1 rounded-[10px] border-0 px-3.5 py-3 text-start transition-shadow duration-200 ${
              on ? "bg-accent-bg shadow-[0_0_0_1.5px_var(--color-accent)]" : "bg-raise shadow-[0_0_0_1px_var(--color-line)] hover:shadow-[0_0_0_1px_var(--color-line-strong)]"
            }`}
          >
            <span className="font-semibold">{m.label}</span>
            <span
              className={`col-start-2 row-span-2 row-start-1 grid size-[18px] place-items-center self-center rounded-full ${
                on ? "bg-accent" : "shadow-[inset_0_0_0_1.5px_#4a4d55]"
              }`}
            >
              {on && <span className="size-1.5 rounded-full bg-bg" />}
            </span>
            <span className="text-xs text-muted">{m.value ? (MODEL_DESC[m.value] ? t(MODEL_DESC[m.value]!) : "") : t("Herda do nível de cima.")}</span>
          </button>
        );
      })}
    </div>
  );
}

function EffortPicker({ value, onChange, efforts, allowInherit }: {
  value: string;
  onChange: (v: string) => void;
  efforts: string[];
  allowInherit: boolean;
}) {
  const opts = allowInherit ? ["", ...efforts] : efforts;
  return (
    <div className={`grid gap-1.5 ${allowInherit ? "grid-cols-6" : "grid-cols-5"}`} role="radiogroup" aria-label={t("Esforço")}>
      {opts.map((e) => {
        const on = e === value;
        const lvl = EFFORT_LEVEL[e] ?? 0;
        return (
          <button
            key={e || "inherit"}
            type="button"
            role="radio"
            aria-checked={on}
            onClick={() => onChange(e)}
            className={`flex h-[84px] flex-col items-center justify-end gap-2 rounded-[10px] border-0 px-1 py-2.5 text-xs font-medium transition-shadow duration-200 ${
              on ? "bg-accent-bg text-fg shadow-[0_0_0_1.5px_var(--color-accent)]" : "bg-raise text-muted shadow-[0_0_0_1px_var(--color-line)] hover:text-fg"
            }`}
          >
            <span className="flex h-[30px] items-end gap-[3px]" aria-hidden>
              {[1, 2, 3, 4, 5].map((i) => (
                <span key={i} className={`w-[5px] rounded-[2px] ${on && i <= lvl ? "bg-accent" : i <= lvl ? "bg-[#5a5d66]" : "bg-line-strong"}`} style={{ height: 6 + i * 5 }} />
              ))}
            </span>
            {e ? effortLabel(e) : t("padrão")}
          </button>
        );
      })}
    </div>
  );
}

function ConfigDialog({ target, cfg, onClose }: { target: Target; cfg: AgentConfig; onClose: () => void }) {
  const qc = useQueryClient();
  const [pending, start] = useTransition();
  const init =
    target.kind === "global"
      ? { model: cfg.model, effort: cfg.effort }
      : target.kind === "task"
        ? { model: target.task.model, effort: target.task.effort }
        : { model: target.ch.model, effort: target.ch.effort };
  const [model, setModel] = useState(init.model);
  const [effort, setEffort] = useState(init.effort);
  const [parallel, setParallel] = useState(cfg.parallel);
  const [own, setOwn] = useState(target.kind === "channel" ? !!(init.model || init.effort) : true);
  const [tasks, setTasks] = useState<TaskRow[]>(target.kind === "channel" ? target.ch.tasks : []);
  const inherit = target.kind !== "global";

  const title =
    target.kind === "global" ? t("Padrão pra todos") : target.kind === "task" ? tx(target.task.label) : t("Agente do {slug}", { slug: target.ch.slug });
  const sub =
    target.kind === "global"
      ? t("Vale pra toda tarefa de todo canal que não tiver ajuste.")
      : target.kind === "task"
        ? t("{d}. Vale pra todos os canais.", { d: TASK_DESC[target.task.task] ? t(TASK_DESC[target.task.task]!) : "" })
        : t("Ajuste do canal vale mais que o da tarefa e o padrão.");

  const save = () =>
    start(async () => {
      try {
        if (target.kind === "global") {
          await commands.setAgentConfig({ channel: null, task: null }, model, effort);
          if (parallel !== cfg.parallel) await commands.setParallel(parallel);
        } else if (target.kind === "task") {
          await commands.setAgentConfig({ channel: null, task: target.task.task }, model, effort);
        } else {
          const slug = target.ch.slug;
          await commands.setAgentConfig({ channel: slug, task: null }, own ? model : "", own ? effort : "");
          for (const tk of tasks) {
            const before = target.ch.tasks.find((x) => x.task === tk.task);
            if (before && (before.model !== tk.model || before.effort !== tk.effort))
              await commands.setAgentConfig({ channel: slug, task: tk.task }, tk.model, tk.effort);
          }
        }
        toast(t("Agente atualizado"));
        await qc.invalidateQueries({ queryKey: keys.agents });
        await qc.invalidateQueries({ queryKey: keys.status });
        await qc.invalidateQueries({ queryKey: keys.channels });
        await qc.invalidateQueries({ queryKey: ["channel"] });
        onClose();
      } catch (e) {
        toast(errMsg(e), "error");
      }
    });

  return (
    <div className="flex max-h-[calc(100vh-80px)] flex-col">
      <div className="flex items-center gap-3.5 px-[22px] pt-5 pb-4">
        {target.kind === "channel" && <Avatar slug={target.ch.slug} color={target.ch.color} size={40} />}
        <div className="flex min-w-0 flex-1 flex-col">
          <h2 className="m-0 text-lg font-semibold">{title}</h2>
          <span className="text-[13px] text-muted">{sub}</span>
        </div>
      </div>
      <div className="overflow-y-auto">
        {target.kind === "channel" && (
          <div className="flex flex-col gap-3 border-t border-line-soft px-[22px] py-[18px]">
            <div className="grid grid-cols-2 gap-2 rounded-xl bg-bg p-1 shadow-[inset_0_0_0_1px_var(--color-line-soft)]" role="radiogroup" aria-label={t("Como configurar")}>
              {[
                { v: false, t: t("Seguir o padrão"), d: summaryOf("", "", cfg) },
                { v: true, t: t("Configurar próprio"), d: t("Modelo e esforço só desse canal") },
              ].map((o) => (
                <button
                  key={String(o.v)}
                  type="button"
                  role="radio"
                  aria-checked={own === o.v}
                  onClick={() => setOwn(o.v)}
                  className={`flex flex-col gap-0.5 rounded-[9px] border-0 px-3 py-2.5 text-start ${
                    own === o.v ? "bg-[#22233d] text-fg shadow-[0_0_0_1px_#3d3f7a]" : "bg-transparent text-muted hover:text-fg"
                  }`}
                >
                  <span className="font-medium">{o.t}</span>
                  <span className="text-xs text-dim">{o.d}</span>
                </button>
              ))}
            </div>
          </div>
        )}
        {(target.kind !== "channel" || own) && (
          <>
            <div className="flex flex-col gap-3 border-t border-line-soft px-[22px] py-[18px]">
              <span className="text-[13px] font-semibold">{t("Modelo")}</span>
              <ModelPicker value={model} onChange={setModel} models={cfg.models} allowInherit={inherit} />
            </div>
            <div className="flex flex-col gap-3 border-t border-line-soft px-[22px] py-[18px]">
              <span className="text-[13px] font-semibold">{t("Esforço")}</span>
              <span className="-mt-2 text-xs text-dim">{t("Mais esforço pensa mais antes de responder: melhor resultado, mais tokens.")}</span>
              <EffortPicker value={effort} onChange={setEffort} efforts={cfg.efforts} allowInherit={inherit} />
            </div>
          </>
        )}
        {target.kind === "global" && (
          <div className="flex flex-col gap-2 border-t border-line-soft px-[22px] py-[18px]">
            <span className="text-[13px] font-semibold">{t("Agentes ao mesmo tempo")}</span>
            <span className="text-xs text-dim">{t("Quantos agentes podem pensar juntos. O navegador continua um por vez.")}</span>
            <div className="flex gap-1.5">
              {[1, 2, 3, 4].map((n) => (
                <button
                  key={n}
                  type="button"
                  aria-pressed={parallel === n}
                  onClick={() => setParallel(n)}
                  className={`mono h-9 w-12 rounded-lg border-0 ${parallel === n ? "bg-accent-bg text-fg shadow-[0_0_0_1.5px_var(--color-accent)]" : "bg-raise text-muted shadow-[0_0_0_1px_var(--color-line)]"}`}
                >
                  {n}
                </button>
              ))}
            </div>
            <span className="text-xs text-dim">{t("Muda depois de reabrir o app.")}</span>
          </div>
        )}
        {target.kind === "channel" && (
          <div className="flex flex-col gap-2 border-t border-line-soft px-[22px] py-[18px]">
            <span className="text-[13px] font-semibold">{t("Ajuste por tarefa neste canal")}</span>
            {tasks.map((tk, i) => (
              <div key={tk.task} className="grid grid-cols-[minmax(0,1fr)_130px_120px] items-center gap-2 border-t border-[#1f2125] py-2 first:border-t-0">
                <span className="text-[13px]">{tx(tk.label)}</span>
                <Select
                  size="sm"
                  label={t("Modelo: {l}", { l: tx(tk.label) })}
                  value={tk.model}
                  onChange={(v) => setTasks(tasks.map((x, j) => (j === i ? { ...x, model: v } : x)))}
                  options={[{ value: "", label: t("padrão") }, ...cfg.models.map((m) => ({ value: m.value, label: m.label }))]}
                />
                <Select
                  size="sm"
                  label={t("Esforço: {l}", { l: tx(tk.label) })}
                  value={tk.effort}
                  onChange={(v) => setTasks(tasks.map((x, j) => (j === i ? { ...x, effort: v } : x)))}
                  options={[{ value: "", label: t("padrão") }, ...cfg.efforts.map((e) => ({ value: e, label: effortLabel(e) }))]}
                />
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="flex items-center justify-end gap-2 border-t border-line-soft px-[22px] py-4">
        <button className="btn btn-ghost" type="button" onClick={onClose}>
          {t("Cancelar")}
        </button>
        <button className="btn btn-pri" type="button" onClick={save} disabled={pending}>
          {pending && <ISpin size={14} />}
          {t("Salvar")}
        </button>
      </div>
    </div>
  );
}

function Badge({ own, children }: { own: boolean; children: string }) {
  return (
    <span className={`rounded-[5px] px-[7px] py-0.5 text-[11px] font-medium whitespace-nowrap ${own ? "bg-accent-soft text-accent-ink" : "bg-[#1f2125] text-muted"}`}>
      {children}
    </span>
  );
}

function NowCell({ slug }: { slug: string }) {
  const st = useLive((s) => activeStep(s.agents[slug]));
  return (
    <span className="flex items-center gap-2 text-[13px] whitespace-nowrap text-fg-2">
      <span className={`size-[7px] rounded-full ${st ? "anim-pulse bg-accent" : "bg-faint"}`} />
      {st ? tx(st.label) : t("parado")}
    </span>
  );
}

const ROW = "grid grid-cols-[minmax(200px,1.1fr)_minmax(220px,1.3fr)_minmax(160px,1fr)_auto] items-center gap-6 border-t border-line-soft px-5 py-4 first:border-t-0 max-[1100px]:grid-cols-[minmax(0,1fr)_auto]";

export default function Agents() {
  const { data: cfg } = useAgentConfig();
  const { data: channels } = useChannelsLoose();
  const [target, setTarget] = useState<Target | null>(null);
  const niche = (slug: string) => channels?.find((c) => c.slug === slug)?.niche ?? "";

  return (
    <Page>
      <PageHeader title={t("Agentes")} sub={t("Cada canal tem um agente. Todos seguem o padrão, a não ser que você configure um diferente.")} />

      <section className="panel flex flex-wrap items-center gap-x-6 gap-y-3 px-5 py-[18px]" aria-labelledby="t-pad">
        <div className="flex min-w-0 flex-[1_1_260px] flex-col gap-1">
          <h2 id="t-pad" className="m-0 text-base font-semibold">
            {t("Padrão pra todos")}
          </h2>
          <span className="flex items-center gap-2 text-[13px] text-muted">
            {summaryOf(cfg.model, cfg.effort, cfg)}
            <EffortBars level={EFFORT_LEVEL[cfg.effort] ?? 3} />
          </span>
        </div>
        <span className="text-[13px] text-muted">
          {cfg.parallel === 1
            ? t("{n} agente pensando por vez · um navegador por vez, pra proteger as contas.", { n: cfg.parallel })
            : t("{n} agentes pensando por vez · um navegador por vez, pra proteger as contas.", { n: cfg.parallel })}
        </span>
        <button className="btn btn-sec" type="button" onClick={() => setTarget({ kind: "global" })}>
          {t("Editar padrão")}
        </button>
      </section>

      <Panel id="tasks" title={t("Tarefas, pra todos os canais")} sub={t("Cada parte do trabalho pode usar um modelo e esforço diferente.")}>
        <div>
          {cfg.tasks.map((tk, i) => {
            const own = !!(tk.model || tk.effort);
            return (
              <div key={tk.task} className={ROW}>
                <div className="flex min-w-0 items-center gap-3.5">
                  <span aria-hidden className="mono grid size-7 flex-none place-items-center rounded-lg bg-[#1f2125] text-xs text-muted">
                    {i + 1}
                  </span>
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">{tx(tk.label)}</span>
                    <span className="truncate text-[13px] text-muted">{TASK_DESC[tk.task] ? t(TASK_DESC[tk.task]!) : ""}</span>
                  </div>
                </div>
                <span className="flex flex-wrap items-center gap-2 font-medium max-[1100px]:hidden">
                  {summaryOf(tk.model, tk.effort, cfg)}
                  <Badge own={own}>{own ? t("ajustado") : t("padrão")}</Badge>
                </span>
                <span className="max-[1100px]:hidden" />
                <button className="btn btn-sec" type="button" onClick={() => setTarget({ kind: "task", task: tk })}>
                  {t("Configurar")}
                </button>
              </div>
            );
          })}
        </div>
        <div className="border-t border-line-soft px-5 py-3 text-xs text-dim">
          {t("Quem manda, do mais forte pro mais fraco: ajuste da tarefa no canal, depois o canal, depois a tarefa, depois o padrão.")}
        </div>
      </Panel>

      <Panel id="ags" title={t("Agentes dos canais")} sub={t("Clique em Configurar pra mudar o agente de um canal.")}>
        <div>
          {cfg.channels.map((c) => {
            const own = !!(c.model || c.effort);
            const adjusted = c.tasks.filter((tk) => tk.model || tk.effort);
            return (
              <div key={c.slug} className={ROW}>
                <div className="flex min-w-0 items-center gap-3.5">
                  <Avatar slug={c.slug} color={c.color} hasPhoto={channels?.find((x) => x.slug === c.slug)?.has_avatar} size={40} />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">{c.slug}</span>
                    <span className="truncate text-[13px] text-muted">{niche(c.slug)}</span>
                  </div>
                </div>
                <div className="flex min-w-0 flex-col gap-1 max-[1100px]:hidden">
                  <span className="flex flex-wrap items-center gap-2 font-medium">
                    {summaryOf(c.model, c.effort, cfg)}
                    <Badge own={own}>{own ? t("próprio") : t("padrão")}</Badge>
                  </span>
                  {adjusted.length > 0 && (
                    <span className="text-[13px] text-muted">
                      {adjusted.map((tk) => `${tx(tk.label)}: ${modelLabel(tk.model || c.model || cfg.model)}`).join(" · ")}
                    </span>
                  )}
                </div>
                <span className="max-[1100px]:hidden">
                  <NowCell slug={c.slug} />
                </span>
                <button className="btn btn-sec" type="button" onClick={() => setTarget({ kind: "channel", ch: c })}>
                  {t("Configurar")}
                </button>
              </div>
            );
          })}
        </div>
      </Panel>

      <Dialog open={!!target} onClose={() => setTarget(null)} label={t("Configurar agente")}>
        {target && <ConfigDialog target={target} cfg={cfg} onClose={() => setTarget(null)} />}
      </Dialog>
    </Page>
  );
}
