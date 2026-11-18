// Componentes base: Panel, Avatar, Dot, Strip, Switch, Segmented, Dialog, Skeleton, Section header.
import { useEffect, useRef, type ReactNode } from "react";
import type { Stage } from "../lib/api";
import { useAvatar } from "../lib/queries";
import { t, tx } from "../lib/i18n";

export function Panel({ title, sub, actions, children, className = "", id }: {
  title?: ReactNode;
  sub?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
  id?: string;
}) {
  const hid = id ? `${id}-t` : undefined;
  return (
    <section className={`panel min-w-0 ${className}`} aria-labelledby={hid}>
      {(title || sub || actions) && (
        <div className="flex flex-wrap items-start gap-3 px-5 pt-[18px] pb-3.5">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            {title && (
              <h2 id={hid} className="m-0 text-base font-semibold">
                {title}
              </h2>
            )}
            {sub && <p className="m-0 text-[13px] text-muted">{sub}</p>}
          </div>
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

export function Avatar({ slug, color, hasPhoto = false, size = 22, ring }: {
  slug: string;
  color: string;
  hasPhoto?: boolean;
  size?: number;
  ring?: string;
}) {
  const { data } = useAvatar(slug, hasPhoto);
  const ini = slug.replace(/^_/, "").charAt(0) || "_";
  const shadow = ring ? `0 0 0 2px var(--color-panel), 0 0 0 4px ${ring}` : "0 0 0 1px rgba(255,255,255,.08)";
  const style = { width: size, height: size, background: color, boxShadow: shadow, fontSize: Math.round(size * 0.4) };
  if (data) {
    return <img src={data} alt="" className="flex-none rounded-full object-cover" style={style} />;
  }
  return (
    <span aria-hidden className="inline-grid flex-none place-items-center rounded-full font-semibold text-fg" style={style}>
      {slug.startsWith("_") ? "_" : ini}
    </span>
  );
}

const TONE: Record<string, string> = {
  run: "bg-accent",
  ok: "bg-ok",
  warn: "bg-warn",
  error: "bg-live",
  idle: "bg-faint",
  off: "bg-faint",
};

export function Dot({ tone, pulse = false, className = "" }: { tone: string; pulse?: boolean; className?: string }) {
  return <span aria-hidden className={`inline-block size-[7px] flex-none rounded-full ${TONE[tone] ?? "bg-faint"} ${pulse ? "anim-pulse" : ""} ${className}`} />;
}

export function Strip({ stages, labels = false }: { stages: Stage[]; labels?: boolean }) {
  const now = stages.findIndex((s) => s.state === "now");
  const aria = now >= 0 ? t("Agora: {label}, passo {n} de {total}", { label: tx(stages[now]!.label), n: now + 1, total: stages.length }) : t("Etapas do dia");
  if (!labels) {
    return (
      <span className="seg-strip mt-0.5" role="img" aria-label={aria}>
        {stages.map((s) => (
          <i key={s.key} data-s={s.state} />
        ))}
      </span>
    );
  }
  return (
    <div className="flex max-w-[640px] flex-[1_1_520px] gap-1" role="img" aria-label={aria}>
      {stages.map((s) => (
        <span key={s.key} className="flex min-w-0 flex-1 flex-col gap-1.5">
          <span className="seg-strip">
            <i data-s={s.state} />
          </span>
          <span
            className={`truncate text-xs ${
              s.state === "now" ? "font-semibold text-fg" : s.state === "done" || s.state === "posted" ? "text-muted" : "text-dim"
            }`}
          >
            {tx(s.label)}
          </span>
        </span>
      ))}
    </div>
  );
}

export function Switch({ checked, onChange, label, disabled }: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <input
      className="sw"
      type="checkbox"
      role="switch"
      aria-label={label}
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.currentTarget.checked)}
    />
  );
}

export function Segmented<T extends string | number>({ value, options, onChange, label }: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <span role="group" aria-label={label} className="inline-flex rounded-lg bg-bg p-0.5 shadow-[inset_0_0_0_1px_var(--color-line-soft)]">
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          aria-pressed={o.value === value}
          onClick={() => onChange(o.value)}
          className={`h-6 rounded-md border-0 px-2.5 text-xs font-medium transition-colors ${
            o.value === value ? "bg-line text-fg" : "bg-transparent text-dim hover:text-fg"
          }`}
        >
          {o.label}
        </button>
      ))}
    </span>
  );
}

/** `<dialog>` nativo: foco preso e Esc pelo navegador. */
export function Dialog({ open, onClose, children, label }: {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  label: string;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      className="modal m-auto"
      aria-label={label}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {open && children}
    </dialog>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <span aria-hidden className={`skel block ${className}`} />;
}

export function PageHeader({ title, sub, children }: { title: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <header className="flex flex-wrap items-end gap-4">
      <div className="flex flex-[1_1_320px] flex-col gap-1">
        <h1 className="m-0 text-2xl font-semibold tracking-[-0.02em]">{title}</h1>
        {sub && <p className="m-0 text-muted">{sub}</p>}
      </div>
      {children}
    </header>
  );
}

export function Page({ children }: { children: ReactNode }) {
  return <div className="flex max-w-[1120px] flex-col gap-7">{children}</div>;
}

export function EffortBars({ level, max = 5 }: { level: number; max?: number }) {
  return (
    <span aria-hidden className="inline-flex h-3 items-end gap-0.5">
      {Array.from({ length: max }, (_, i) => (
        <span key={i} className={`w-[3px] rounded-[1px] ${i < level ? "bg-accent-ink" : "bg-line-strong"}`} style={{ height: 4 + i * 2 }} />
      ))}
    </span>
  );
}

export function Kv({ k, children }: { k: ReactNode; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2.5 border-t border-[#1f2125] py-2.5 first:border-t-0">
      <span>{k}</span>
      {children}
    </div>
  );
}
