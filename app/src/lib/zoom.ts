import { getCurrentWebview } from "@tauri-apps/api/webview";

// Ctrl + "+" / Ctrl + "-" / Ctrl + 0: zoom da interface inteira, lembrado entre aberturas.
const STEPS = [0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
const KEY = "tkd.zoom";

function read(): number {
  try {
    const v = Number(localStorage.getItem(KEY));
    return STEPS.includes(v) ? v : 1;
  } catch {
    return 1;
  }
}

let level = read();

function apply(z: number) {
  level = z;
  try {
    localStorage.setItem(KEY, String(z));
  } catch {
    /* sem storage: vale so nesta sessao */
  }
  getCurrentWebview().setZoom(z).catch(() => {});
}

export function startZoom() {
  if (level !== 1) apply(level);
  window.addEventListener(
    "keydown",
    (e) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const i = STEPS.indexOf(level);
      let next: number | null = null;
      if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") next = STEPS[Math.min(i + 1, STEPS.length - 1)] ?? null;
      else if (e.key === "-" || e.key === "_" || e.code === "NumpadSubtract") next = STEPS[Math.max(i - 1, 0)] ?? null;
      else if (e.key === "0" || e.code === "Numpad0") next = 1;
      if (next === null) return;
      e.preventDefault();
      if (next !== level) apply(next);
    },
    { capture: true },
  );
}
