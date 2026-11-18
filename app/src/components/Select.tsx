// Select do app: botao no visual dos campos + lista escura (o <select> nativo do Windows ignora o tema).
// A lista abre como popover (camada do topo): nao e cortada por dialog nem por painel com scroll.
// Teclado: setas, Home/End, Enter/Espaco, Esc, Tab e digitar a primeira letra.
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { ICheck } from "./icons";
import { isRtl } from "../lib/i18n";

export type SelectOption<T extends string> = { value: T; label: string; hint?: string };

export function Select<T extends string>({ value, options, onChange, label, size = "md", className = "", placeholder = "" }: {
  value: T;
  options: SelectOption<T>[];
  onChange: (v: T) => void;
  /** nome acessivel (o label visivel do campo, se houver) */
  label: string;
  size?: "md" | "sm";
  className?: string;
  placeholder?: string;
}) {
  const id = useId();
  const btn = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const typed = useRef({ text: "", at: 0 });
  const current = options.find((o) => o.value === value);
  const hasHints = options.some((o) => o.hint);

  const show = (at = Math.max(0, options.findIndex((o) => o.value === value))) => {
    setActive(at);
    setOpen(true);
  };
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) btn.current?.focus();
  };
  const pick = (i: number) => {
    const o = options[i];
    if (o) onChange(o.value);
    close();
  };

  // abre/fecha o popover e posiciona embaixo do botao (ou em cima, se nao couber)
  useLayoutEffect(() => {
    const p = pop.current;
    const b = btn.current;
    if (!p || !b) return;
    if (!open) {
      if (p.matches(":popover-open")) p.hidePopover();
      return;
    }
    if (!p.matches(":popover-open")) p.showPopover();
    const place = () => {
      const r = b.getBoundingClientRect();
      const h = p.offsetHeight;
      const below = window.innerHeight - r.bottom - 8;
      const up = below < h && r.top > below;
      const w = Math.max(r.width, p.offsetWidth);
      const x = isRtl() ? r.right - w : r.left;
      p.style.left = `${Math.max(8, Math.min(x, window.innerWidth - w - 8))}px`;
      p.style.top = `${up ? r.top - h - 6 : r.bottom + 6}px`;
      p.style.minWidth = `${r.width}px`;
      p.dataset.side = up ? "top" : "bottom";
    };
    place();
    list.current?.focus();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  // opcao ativa sempre visivel
  useEffect(() => {
    if (open) list.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  const onListKey = (e: React.KeyboardEvent) => {
    const n = options.length;
    const k = e.key;
    if (k === "ArrowDown") setActive((a) => (a + 1) % n);
    else if (k === "ArrowUp") setActive((a) => (a - 1 + n) % n);
    else if (k === "Home") setActive(0);
    else if (k === "End") setActive(n - 1);
    else if (k === "Enter" || k === " ") pick(active);
    else if (k === "Escape") close();
    else if (k === "Tab") {
      close(false);
      return; // deixa o Tab seguir
    } else if (k.length === 1 && /\S/.test(k)) {
      const now = Date.now();
      typed.current = { text: (now - typed.current.at < 700 ? typed.current.text : "") + k.toLowerCase(), at: now };
      const t = typed.current.text;
      const i = options.findIndex((o, j) => j > (t.length === 1 ? active : active - 1) && o.label.toLowerCase().startsWith(t));
      const any = i >= 0 ? i : options.findIndex((o) => o.label.toLowerCase().startsWith(t));
      if (any >= 0) setActive(any);
    } else return;
    e.preventDefault();
  };

  const h = size === "sm" ? "h-8 text-[13px] px-2.5" : "h-9 text-sm px-3";
  return (
    <>
      <button
        ref={btn}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-list`}
        aria-label={label}
        onClick={() => (open ? close() : show())}
        onKeyDown={(e) => {
          if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
            e.preventDefault();
            show();
          }
        }}
        className={`sel-trigger flex w-full min-w-0 items-center justify-between gap-2 rounded-lg border border-line-strong bg-well text-start text-fg transition-[border-color,box-shadow] ${h} ${className}`}
        data-open={open || undefined}
      >
        <span className={`truncate ${current ? "" : "text-faint"}`}>{current?.label ?? placeholder}</span>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden className="sel-chev shrink-0 text-dim">
          <path d="m4 6 4 4 4-4" />
        </svg>
      </button>
      <div ref={pop} popover="manual" className="sel-pop" onToggle={(e) => (e as unknown as ToggleEvent).newState === "closed" && open && setOpen(false)}>
        <ul
          ref={list}
          id={`${id}-list`}
          role="listbox"
          aria-label={label}
          tabIndex={-1}
          aria-activedescendant={open ? `${id}-o${active}` : undefined}
          onKeyDown={onListKey}
          onBlur={(e) => {
            if (!pop.current?.contains(e.relatedTarget as Node) && e.relatedTarget !== btn.current) close(false);
          }}
          className="m-0 flex max-h-[min(320px,60vh)] list-none flex-col gap-0.5 overflow-y-auto p-1 outline-none"
        >
          {options.map((o, i) => {
            const sel = o.value === value;
            return (
              <li
                key={o.value}
                id={`${id}-o${i}`}
                data-i={i}
                role="option"
                aria-selected={sel}
                onMouseMove={() => active !== i && setActive(i)}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(i)}
                className={`flex cursor-pointer items-start gap-2.5 rounded-md px-2.5 ${hasHints ? "py-2" : "py-1.5"} ${
                  i === active ? "bg-hover" : ""
                } ${size === "sm" ? "text-[13px]" : "text-sm"}`}
              >
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className={sel ? "font-medium text-fg" : "text-fg-2"}>{o.label}</span>
                  {o.hint && <span className="text-xs text-dim">{o.hint}</span>}
                </span>
                <ICheck size={14} className={`mt-[3px] shrink-0 text-accent ${sel ? "" : "invisible"}`} />
              </li>
            );
          })}
        </ul>
      </div>
    </>
  );
}
