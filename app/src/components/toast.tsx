// Avisos curtos no canto (sucesso / erro). Store minima com useSyncExternalStore.
import { useSyncExternalStore } from "react";

type Toast = { id: number; text: string; tone: "ok" | "error" };
let items: Toast[] = [];
let seq = 0;
const ls = new Set<() => void>();
const emit = () => ls.forEach((l) => l());

export function toast(text: string, tone: "ok" | "error" = "ok") {
  const id = ++seq;
  items = [...items, { id, text, tone }].slice(-4);
  emit();
  setTimeout(() => {
    items = items.filter((t) => t.id !== id);
    emit();
  }, tone === "error" ? 7000 : 3200);
}

export function Toaster() {
  const list = useSyncExternalStore(
    (f) => (ls.add(f), () => ls.delete(f)),
    () => items,
  );
  return (
    <div aria-live="polite" className="fixed bottom-5 end-5 z-50 flex w-[min(380px,calc(100vw-40px))] flex-col gap-2">
      {list.map((t) => (
        <div
          key={t.id}
          role={t.tone === "error" ? "alert" : "status"}
          className="anim-rise flex items-start gap-2.5 rounded-[10px] bg-raise px-3.5 py-3 text-[13px] shadow-[0_0_0_1px_#2e3036,0_12px_32px_rgba(0,0,0,.5)]"
        >
          <span className={`mt-1.5 size-[7px] flex-none rounded-full ${t.tone === "error" ? "bg-live" : "bg-ok"}`} />
          <span className="min-w-0 break-words">{t.text}</span>
        </div>
      ))}
    </div>
  );
}
