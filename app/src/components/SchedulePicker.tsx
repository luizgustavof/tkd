// "Quando postar": dias da semana + horarios em 4 periodos + variacao. Salva na hora (otimista).
import { useOptimistic, useState, useTransition, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { commands, errMsg, type ScheduleDto } from "../lib/api";
import { keys } from "../lib/queries";
import { toast } from "./toast";
import { IMoon, IPlus, ISun, ISunrise, ISunset, IX } from "./icons";
import { Segmented, Switch } from "./ui";
import { t, tx } from "../lib/i18n";
import type { MsgKey } from "../lib/messages";

const DAYS = ["seg", "ter", "qua", "qui", "sex", "sab", "dom"] as const;
const WEEKDAYS = ["seg", "ter", "qua", "qui", "sex"];
const WEEKEND = ["sab", "dom"];

const PERIODS: { key: string; label: MsgKey; range: string; from: number; to: number; def: string; icon: ReactNode }[] = [
  { key: "mad", label: "Madrugada", range: "0–6h", from: 0, to: 6, def: "02:00", icon: <IMoon /> },
  { key: "man", label: "Manhã", range: "6–12h", from: 6, to: 12, def: "09:00", icon: <ISunrise /> },
  { key: "tar", label: "Tarde", range: "12–18h", from: 12, to: 18, def: "15:00", icon: <ISun /> },
  { key: "noi", label: "Noite", range: "18–24h", from: 18, to: 24, def: "20:00", icon: <ISunset /> },
];

const hourOf = (t: string) => Number(t.slice(0, 2));
const norm = (s: ScheduleDto): ScheduleDto => ({ ...s, times: [...new Set(s.times)].sort() });

export function SchedulePicker({ slug, schedule, nextLabel }: { slug: string; schedule: ScheduleDto; nextLabel: string }) {
  const qc = useQueryClient();
  const [opt, setOpt] = useOptimistic(schedule);
  const [, start] = useTransition();
  const [adding, setAdding] = useState<string | null>(null);

  const save = (next: ScheduleDto) =>
    start(async () => {
      const n = norm(next);
      setOpt(n);
      try {
        await commands.setSchedule(slug, n);
        await Promise.all([
          qc.invalidateQueries({ queryKey: keys.channel(slug) }),
          qc.invalidateQueries({ queryKey: keys.channels }),
        ]);
      } catch (e) {
        toast(errMsg(e), "error"); // o otimista volta sozinho para o valor salvo
      }
    });

  const days = opt.days.length ? opt.days : [...DAYS];
  const allDays = days.length === 7;
  const preset = allDays ? "all" : days.join() === WEEKDAYS.join() ? "week" : days.join() === WEEKEND.join() ? "end" : "";
  const setDays = (d: string[]) => save({ ...opt, days: d.length === 7 ? [] : DAYS.filter((x) => d.includes(x)) });
  const toggleDay = (d: string) => {
    const next = days.includes(d) ? days.filter((x) => x !== d) : [...days, d];
    if (!next.length) return toast(t("Deixe pelo menos um dia"), "error");
    setDays(next);
  };
  const removeTime = (tm: string) => {
    if (opt.times.length <= 1) return toast(t("Deixe pelo menos um horário"), "error");
    save({ ...opt, times: opt.times.filter((x) => x !== tm) });
  };
  const perWeek = days.length * opt.times.length;

  return (
    <>
      <fieldset className="m-0 flex min-w-0 flex-col gap-2.5 border-0 px-5 pt-0 pb-[18px]">
        <div className="flex items-center gap-2.5">
          <legend className="float-start p-0 text-[13px] font-medium text-fg-2">{t("Dias")}</legend>
          <span className="ms-auto">
            <Segmented
              label={t("Atalhos de dias")}
              value={preset}
              onChange={(v) => setDays(v === "all" ? [...DAYS] : v === "week" ? WEEKDAYS : WEEKEND)}
              options={[
                { value: "all", label: t("Todo dia") },
                { value: "week", label: t("Úteis") },
                { value: "end", label: t("Fds") },
              ]}
            />
          </span>
        </div>
        <div className="clear-both grid grid-cols-7 gap-1 rounded-xl bg-bg p-1 shadow-[inset_0_0_0_1px_var(--color-line-soft)]">
          {DAYS.map((d) => {
            const on = days.includes(d);
            return (
              <label key={d} className="relative">
                <input type="checkbox" className="pointer-events-none absolute opacity-0" checked={on} onChange={() => toggleDay(d)} />
                <span
                  className={`flex cursor-pointer flex-col items-center gap-1.5 rounded-[9px] pt-2.5 pb-[9px] text-xs font-medium transition-colors duration-200 after:size-[5px] after:rounded-full after:transition-colors after:content-[''] ${
                    on ? "bg-[#22233d] text-fg after:bg-accent" : "text-dim after:bg-[#2e3036] hover:bg-[#17181b] hover:text-fg"
                  }`}
                >
                  {t(d === "sab" ? "sáb" : d)}
                </span>
              </label>
            );
          })}
        </div>
      </fieldset>

      <div className="flex flex-col gap-3 border-t border-line-soft px-5 pt-4 pb-[18px]">
        <span className="text-[13px] font-medium text-fg-2">{t("Horários")}</span>
        <div className="grid grid-cols-2 gap-2">
          {PERIODS.map((p) => {
            const times = opt.times.filter((t) => hourOf(t) >= p.from && hourOf(t) < p.to);
            const on = times.length > 0;
            return (
              <div
                key={p.key}
                className={`flex min-h-[92px] flex-col justify-between gap-3 rounded-xl p-3 transition-shadow duration-200 ${
                  on ? "bg-accent-bg text-accent-ink shadow-[0_0_0_1.5px_var(--color-accent)]" : "bg-raise text-dim shadow-[0_0_0_1px_var(--color-line)] hover:shadow-[0_0_0_1px_var(--color-line-strong)]"
                }`}
              >
                <div className="flex items-center gap-2">
                  {p.icon}
                  <span className={`text-[13px] font-semibold ${on ? "text-fg" : "text-fg-2"}`}>{t(p.label)}</span>
                  <span className={`mono ms-auto text-[11px] ${on ? "text-[#9a9be0]" : "text-[#6f727d]"}`}>{p.range}</span>
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                  {times.map((tm) => (
                    <span key={tm} className="inline-flex h-[30px] items-center gap-0.5 rounded-lg bg-accent pe-0.5 ps-2.5 text-sm font-medium text-bg">
                      <span className="mono">{tm}</span>
                      <button
                        type="button"
                        aria-label={t("Tirar {t}", { t: tm })}
                        onClick={() => removeTime(tm)}
                        className="inline-grid size-6 place-items-center rounded-md border-0 bg-transparent text-[#2a2b55] hover:bg-[rgba(14,15,17,.12)] hover:text-bg"
                      >
                        <IX size={12} />
                      </button>
                    </span>
                  ))}
                  {adding === p.key ? (
                    <form
                      className="flex items-center gap-1"
                      onSubmit={(e) => {
                        e.preventDefault();
                        const v = String(new FormData(e.currentTarget).get("t") ?? "");
                        setAdding(null);
                        if (/^\d\d:\d\d$/.test(v)) save({ ...opt, times: [...opt.times, v] });
                      }}
                    >
                      <input
                        name="t"
                        type="time"
                        defaultValue={p.def}
                        autoFocus
                        aria-label={t("Novo horário ({p})", { p: t(p.label) })}
                        className="!h-[30px] w-[92px] !px-2"
                        onKeyDown={(e) => e.key === "Escape" && setAdding(null)}
                      />
                      <button className="btn btn-pri btn-sm !h-[30px]" type="submit">
                        ok
                      </button>
                    </form>
                  ) : (
                    <button
                      type="button"
                      aria-label={t("Adicionar horário: {p}", { p: t(p.label).toLowerCase() })}
                      onClick={() => setAdding(p.key)}
                      className={`inline-grid ${on ? "size-[30px]" : "size-8"} place-items-center rounded-lg border border-dashed border-line-strong bg-transparent text-dim transition-colors hover:border-accent hover:text-fg`}
                    >
                      <IPlus size={14} />
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
        <span className="text-xs text-dim">{t("Toque no + do período e escolha a hora.")}</span>
        <label className="flex items-center gap-2.5 pt-0.5 text-[13px] text-fg-2">
          <Switch
            label={t("Variar o horário um pouco")}
            checked={opt.jitter_min > 0}
            onChange={(v) => save({ ...opt, jitter_min: v ? 10 : 0 })}
          />
          <span className="flex-1">{t("Variar o horário um pouco")}</span>
          <span className="mono text-xs text-dim">{opt.jitter_min > 0 ? <bdi>{`± ${opt.jitter_min} min`}</bdi> : t("exato")}</span>
        </label>
      </div>

      <div className="grid grid-cols-[minmax(0,1fr)_112px] items-end gap-4 border-t border-line-soft px-5 pt-3.5 pb-[18px]">
        <div className="flex flex-col gap-0.5">
          <span className="text-[13px] text-muted">{t("Próximo vídeo")}</span>
          <span className="text-[15px] font-semibold">{tx(nextLabel)}</span>
          <span className="text-xs text-dim">
            {perWeek === 1 ? t("{n} vídeo por semana", { n: perWeek }) : t("{n} vídeos por semana", { n: perWeek })}
          </span>
        </div>
        <div className="grid h-[26px] grid-cols-7 items-end gap-1" role="img" aria-label={t("Semana: {n} vídeos", { n: perWeek })}>
          {DAYS.map((d, i) => {
            const today = (new Date().getDay() + 6) % 7 === i;
            const on = days.includes(d);
            return (
              <i
                key={d}
                className={`block rounded-[2px] ${on ? (today ? "bg-fg" : "bg-accent") : today ? "bg-faint" : "bg-line"}`}
                style={{ height: on ? 6 + Math.min(opt.times.length, 3) * 4 : 4 }}
              />
            );
          })}
        </div>
      </div>
    </>
  );
}
