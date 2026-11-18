// Primeira execucao: o que falta instalar, para que serve e como consertar.
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg, type SetupItem } from "../lib/api";
import { keys, useSetup } from "../lib/queries";
import { Page, PageHeader, Panel } from "../components/ui";
import { ICheck, ISpin, IX } from "../components/icons";
import { toast } from "../components/toast";
import { t, tx } from "../lib/i18n";
import type { MsgKey } from "../lib/messages";

const TONE: Record<string, string> = { ok: "text-ok", warn: "text-warn", missing: "text-live" };
const WORD: Record<string, MsgKey> = { ok: "ok", warn: "atenção", missing: "faltando" };

function Copy({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  return (
    <div className="flex min-w-0 items-center gap-2 rounded-md bg-well px-3 py-2">
      <code className="mono min-w-0 flex-1 truncate text-xs" title={text}>
        {text}
      </code>
      <button
        type="button"
        className="btn btn-sec flex-none text-xs"
        onClick={() =>
          navigator.clipboard.writeText(text).then(
            () => {
              setDone(true);
              setTimeout(() => setDone(false), 1500);
            },
            () => toast(t("não consegui copiar"), "error"),
          )
        }
      >
        {done ? t("Copiado") : t("Copiar")}
      </button>
    </div>
  );
}

function Row({ it }: { it: SetupItem }) {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  const fix = async () => {
    if (!it.fix) return;
    setBusy(true);
    try {
      toast(await commands.setupFix(it.fix));
      await qc.invalidateQueries({ queryKey: keys.setup });
    } catch (e) {
      toast(errMsg(e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid grid-cols-[22px_minmax(0,1fr)] gap-3 border-t border-line-soft px-5 py-4 first:border-t-0">
      <span className={`pt-0.5 ${TONE[it.status]}`} aria-hidden>
        {it.status === "ok" ? <ICheck /> : <IX />}
      </span>
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <span className="font-medium">{tx(it.label)}</span>
          <span className={`text-xs ${TONE[it.status]}`}>{WORD[it.status] ? t(WORD[it.status]!) : it.status}</span>
          <span className="mono min-w-0 truncate text-xs text-dim" title={it.detail} dir="auto">
            {it.detail}
          </span>
        </div>
        <span className="text-sm text-muted">{tx(it.why)}</span>
        {it.status !== "ok" && (it.command || it.fix) && (
          <div className="flex flex-wrap items-center gap-2 pt-1">
            {it.fix && (
              <button type="button" className="btn btn-pri text-xs" disabled={busy} onClick={fix}>
                {busy ? <ISpin /> : null}
                {t("Consertar")}
              </button>
            )}
            {it.command && (
              <div className="min-w-0 flex-[1_1_320px]">
                <Copy text={it.command} />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default function Setup() {
  const qc = useQueryClient();
  const { data, isFetching } = useSetup();
  const items = data ?? [];
  const missing = items.filter((i) => i.status === "missing").length;
  const warn = items.filter((i) => i.status === "warn").length;
  const sub = !data
    ? t("Conferindo as ferramentas…")
    : missing
      ? t("{n} item(ns) faltando. Os canais não rodam sem eles.", { n: missing })
      : warn
        ? t("Tudo instalado; {n} aviso(s) para olhar.", { n: warn })
        : t("Tudo pronto. Crie um canal e ligue o piloto automático.");
  return (
    <Page>
      <PageHeader title={t("Primeiros passos")} sub={sub}>
        <button type="button" className="btn btn-sec" disabled={isFetching} onClick={() => qc.invalidateQueries({ queryKey: keys.setup })}>
          {isFetching ? <ISpin /> : null}
          {t("Conferir de novo")}
        </button>
      </PageHeader>
      <Panel title={t("Ferramentas e configuração")} sub={t("Depois de instalar algo pelo terminal, feche e abra o app (o PATH novo só vale para processos novos).")}>
        <div className="pb-1">
          {items.map((it) => (
            <Row key={it.id} it={it} />
          ))}
        </div>
      </Panel>
    </Page>
  );
}
