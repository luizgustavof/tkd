import { useState, useTransition } from "react";
import { Link } from "@tanstack/react-router";
import { useChannelsLoose, useUsage } from "../lib/queries";
import type { TaskCompareDto, UsageRow } from "../lib/api";
import { modelLabel, pct, tokens } from "../lib/format";
import { Avatar, Page, PageHeader, Panel, Segmented } from "../components/ui";
import { ISpark } from "../components/icons";

const INTERACTIVE = "(conversas)";
const TASK_ORDER = ["recreate", "tag_clips", "ideate", "study", "review", "footage"];
const TASK_SHORT: Record<string, string> = {
  recreate: "Recriar",
  tag_clips: "Catalogar",
  ideate: "Ideias",
  study: "Estudar",
  review: "Avaliar",
  footage: "Imagens",
};

function ByChannel({ rows }: { rows: UsageRow[] }) {
  const { data: chans } = useChannelsLoose();
  const total = rows.reduce((s, r) => s + r.total, 0);
  const max = Math.max(1, ...rows.map((r) => r.total));
  const idle = (chans ?? []).filter((c) => !rows.some((r) => r.channel === c.slug));
  const ROW = "grid grid-cols-[150px_minmax(0,1fr)_96px_80px_104px] items-center gap-5 px-5 max-[900px]:grid-cols-[120px_minmax(0,1fr)_80px]";
  return (
    <Panel
      id="can"
      title="Por canal"
      actions={
        <span className="flex flex-wrap gap-4">
          <span className="inline-flex items-center gap-1.5 text-xs text-muted">
            <i className="size-2.5 rounded-[3px] bg-accent" />
            novo (lido e escrito pela primeira vez)
          </span>
          <span className="inline-flex items-center gap-1.5 text-xs text-muted">
            <i className="size-2.5 rounded-[3px] bg-[#45478a]" />
            reaproveitado do cache, bem mais barato
          </span>
        </span>
      }
    >
      <div className="overflow-x-auto pb-2">
        <div className={`${ROW} pt-1 pb-2 text-xs font-medium text-dim`}>
          <span>Canal</span>
          <span />
          <span className="text-right">Total</span>
          <span className="text-right max-[900px]:hidden">Chamadas</span>
          <span className="text-right max-[900px]:hidden">Por chamada</span>
        </div>
        {rows.map((r) => {
          const c = chans?.find((x) => x.slug === r.channel);
          return (
            <div key={r.channel} className={`${ROW} border-t border-line-soft py-3.5`}>
              <span className="flex min-w-0 items-center gap-2.5">
                <Avatar slug={r.channel} color={c?.color ?? "#2b2d31"} hasPhoto={c?.has_avatar} size={30} />
                <span className="truncate">{r.channel}</span>
              </span>
              <div className="flex items-center gap-2.5">
                <div
                  className="flex h-3.5 overflow-hidden rounded bg-raise"
                  style={{ width: `${(r.total / max) * 100}%`, minWidth: 4 }}
                  role="img"
                  aria-label={`${r.channel}: ${tokens(r.fresh)} novos e ${tokens(r.cache_read)} do cache`}
                >
                  <span className="bg-accent" style={{ width: `${(r.fresh / Math.max(1, r.total)) * 100}%` }} />
                  <span className="bg-[#45478a] shadow-[inset_1px_0_0_var(--color-panel)]" style={{ flex: 1 }} />
                </div>
                <span className="mono text-xs text-dim">{pct(r.total, total)}</span>
              </div>
              <span className="mono text-right text-[13px]">{tokens(r.total)}</span>
              <span className="mono text-right text-[13px] max-[900px]:hidden">{r.calls}</span>
              <span className="mono text-right text-[13px] max-[900px]:hidden">{tokens(r.total / Math.max(1, r.calls))}</span>
            </div>
          );
        })}
        {idle.map((c) => (
          <div key={c.slug} className={`${ROW} border-t border-line-soft py-3.5`}>
            <span className="flex min-w-0 items-center gap-2.5">
              <Avatar slug={c.slug} color={c.color} hasPhoto={c.has_avatar} size={30} />
              <span className="truncate">{c.slug}</span>
            </span>
            <span className="text-[13px] text-muted">nenhum agente rodou nesse período</span>
            <span className="mono text-right text-[13px]">0</span>
            <span className="mono text-right text-[13px] max-[900px]:hidden">0</span>
            <span className="mono text-right text-[13px] max-[900px]:hidden">—</span>
          </div>
        ))}
      </div>
    </Panel>
  );
}

function ByTask({ rows }: { rows: UsageRow[] }) {
  const max = Math.max(1, ...rows.map((r) => r.total));
  return (
    <Panel id="tar" title="Por tarefa" sub="somando todos os canais" className="flex-[1_1_380px]">
      <div className="pb-2">
        {rows.map((r) => (
          <div key={r.task} className="grid grid-cols-[150px_minmax(0,1fr)_96px] items-center gap-5 border-t border-line-soft px-5 py-[11px]">
            <span className="truncate">{r.task_label}</span>
            <div>
              <span className="block h-2.5 rounded-[3px] bg-accent" style={{ width: `${(r.total / max) * 100}%`, minWidth: 3 }} />
            </div>
            <span className="mono text-right text-[13px]">{tokens(r.total)}</span>
          </div>
        ))}
        {!rows.length && <p className="m-0 border-t border-line-soft px-5 py-4 text-[13px] text-dim">Nenhuma chamada nesse período.</p>}
      </div>
    </Panel>
  );
}

function BeforeAfter({ rows }: { rows: TaskCompareDto[] }) {
  const list = rows.filter((r) => r.before_calls > 0 || r.after_calls > 0);
  return (
    <Panel
      id="antes"
      title="Antes × depois do conserto"
      sub="Tokens por chamada. Desde 16/11 o agente roda isolado: sem hooks, MCP, skills e memória, só as ferramentas da tarefa."
    >
      <div className="pb-2">
        <div className="grid grid-cols-[150px_repeat(3,minmax(0,1fr))] gap-5 px-5 pb-2 text-xs text-dim">
          <span>Tarefa</span>
          <span className="text-right">Antes</span>
          <span className="text-right">Depois</span>
          <span className="text-right">Mudança</span>
        </div>
        {list.map((r) => {
          const both = r.before_calls > 0 && r.after_calls > 0;
          const cut = both ? Math.round((1 - r.after_avg / r.before_avg) * 100) : null;
          return (
            <div key={r.task} className="grid grid-cols-[150px_repeat(3,minmax(0,1fr))] items-center gap-5 border-t border-line-soft px-5 py-[11px]">
              <span className="truncate">{r.task_label}</span>
              <span className="mono text-right text-[13px]">
                {r.before_calls ? tokens(r.before_avg) : "—"}
                <span className="text-dim"> · {r.before_calls}×</span>
              </span>
              <span className="mono text-right text-[13px]">
                {r.after_calls ? tokens(r.after_avg) : "ainda não rodou"}
                {r.after_calls > 0 && <span className="text-dim"> · {r.after_calls}×</span>}
              </span>
              <span className={`mono text-right text-[13px] ${cut != null && cut > 0 ? "text-ok" : "text-dim"}`}>
                {cut == null ? "—" : cut > 0 ? `−${cut}%` : `+${-cut}%`}
              </span>
            </div>
          );
        })}
        {!list.length && <p className="m-0 border-t border-line-soft px-5 py-4 text-[13px] text-dim">Sem chamadas ainda.</p>}
      </div>
    </Panel>
  );
}

function Insights({ list }: { list: string[] }) {
  return (
    <Panel id="dica" title="O que chama atenção" className="flex-[1_1_380px]">
      <div className="pb-2">
        {list.map((t, i) => (
          <div key={i} className="grid grid-cols-[32px_minmax(0,1fr)] items-start gap-3 border-t border-line-soft px-5 py-3.5">
            <b className="grid size-8 place-items-center rounded-[9px] bg-accent-bg text-accent-ink">
              <ISpark />
            </b>
            <div className="flex flex-col gap-1">
              <span>{t}</span>
              {i === 1 && (
                <Link to="/agentes" className="text-[13px] font-medium">
                  Configurar essa tarefa
                </Link>
              )}
            </div>
          </div>
        ))}
        {!list.length && <p className="m-0 border-t border-line-soft px-5 py-4 text-[13px] text-dim">Sem dados suficientes ainda.</p>}
      </div>
    </Panel>
  );
}

function Matrix({ pairs }: { pairs: UsageRow[] }) {
  const chans = [...new Set(pairs.map((p) => p.channel))];
  const tasks = TASK_ORDER.filter((t) => pairs.some((p) => p.task === t));
  const max = Math.max(1, ...pairs.map((p) => p.total));
  if (!chans.length) return null;
  return (
    <Panel id="mx" title="Canal × tarefa" sub="em mil tokens · quanto mais forte a cor, mais gastou">
      <div className="overflow-x-auto px-2 pb-3">
        <div role="table" aria-label="Tokens por canal e tarefa, em mil" className="grid min-w-[700px]" style={{ gridTemplateColumns: `130px repeat(${tasks.length}, minmax(84px, 1fr))` }}>
          <div role="row" className="contents">
            <span role="columnheader" className="px-3 pt-0.5 pb-2.5 text-xs font-medium text-dim">
              Canal
            </span>
            {tasks.map((t) => (
              <span key={t} role="columnheader" className="px-3 pt-0.5 pb-2.5 text-right text-xs font-medium text-dim">
                {TASK_SHORT[t]}
              </span>
            ))}
          </div>
          {chans.map((c) => (
            <div role="row" key={c} className="contents">
              <span role="rowheader" className="border-t border-line-soft px-3 py-2.5 text-[13px]">
                {c}
              </span>
              {tasks.map((t) => {
                const v = pairs.find((p) => p.channel === c && p.task === t)?.total ?? 0;
                const a = v ? 0.12 + (v / max) * 0.6 : 0;
                return (
                  <span
                    role="cell"
                    key={t}
                    className={`mono border-t border-line-soft px-3 py-2.5 text-right text-[13px] ${v ? "" : "text-[#4a4d55]"}`}
                    style={v ? { background: `rgba(139,140,255,${a.toFixed(3)})` } : undefined}
                  >
                    {v ? Math.round(v / 1000).toLocaleString("pt-BR") : "—"}
                  </span>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </Panel>
  );
}

export default function Usage() {
  const [days, setDays] = useState(7);
  const [shown, setShown] = useState(7);
  const [pending, start] = useTransition();
  const { data } = useUsage(shown);
  const rows = data.by_channel.filter((r) => r.channel !== INTERACTIVE);
  const pairs = data.by_pair.filter((r) => r.channel !== INTERACTIVE);
  const interactive = data.by_channel.find((r) => r.channel === INTERACTIVE);
  const chTotal = rows.reduce((s, r) => s + r.total, 0);
  const chCalls = rows.reduce((s, r) => s + r.calls, 0);
  const models = data.by_model.filter((m) => m.task).map((m) => modelLabel(m.task));
  const byTask = (() => {
    const m = new Map<string, UsageRow>();
    for (const p of pairs) {
      const r = m.get(p.task);
      m.set(p.task, r ? { ...r, total: r.total + p.total, calls: r.calls + p.calls } : { ...p });
    }
    return [...m.values()].sort((a, b) => b.total - a.total);
  })();

  return (
    <Page>
      <PageHeader
        title="Consumo"
        sub={`Quantos tokens os agentes de cada canal gastaram.${models.length ? ` Modelos usados: ${models.join(", ")}.` : ""}`}
      >
        <div className="flex flex-col items-end gap-1">
          <span className="mono text-2xl font-medium tracking-[-0.02em]">{tokens(chTotal)}</span>
          <span className="text-[13px] text-muted">tokens em {chCalls} chamadas</span>
        </div>
      </PageHeader>
      <div className={`flex items-center gap-3 transition-opacity ${pending ? "opacity-60" : ""}`}>
        <Segmented
          label="Período"
          value={days}
          onChange={(v) => {
            setDays(v);
            start(() => setShown(v));
          }}
          options={[
            { value: 1, label: "Hoje" },
            { value: 7, label: "7 dias" },
            { value: 30, label: "30 dias" },
            { value: 0, label: "Tudo" },
          ]}
        />
      </div>

      <div className={`flex flex-col gap-7 transition-opacity ${pending ? "opacity-60" : ""}`}>
        <ByChannel rows={rows} />
        <div className="flex flex-wrap items-start gap-6">
          <ByTask rows={byTask} />
          <Insights list={data.insights} />
        </div>
        <BeforeAfter rows={data.before_after} />
        <Matrix pairs={pairs} />
        <p className="m-0 max-w-[80ch] text-[13px] text-dim">
          Medido nas chamadas que os agentes fazem e no histórico que o Claude Code deixa salvo no seu PC.
          {interactive
            ? ` Não entra aqui o que você usa conversando direto nesta pasta (${tokens(interactive.total)} no período, ${pct(interactive.cache_read, interactive.total)} cache), porque isso não é gasto dos canais.`
            : ""}
        </p>
      </div>
    </Page>
  );
}
