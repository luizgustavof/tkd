/** 5,70 mi · 166 mil · 820 */
export function tokens(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2).replace(".", ",")} bi`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 1 : 2).replace(".", ",")} mi`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} mil`;
  return String(Math.round(n));
}

export function pct(part: number, total: number): string {
  if (!total) return "0%";
  return `${Math.round((part * 100) / total)}%`;
}

/** "19:42" da hora local de um rfc3339 */
export function hm(rfc: string): string {
  const d = new Date(rfc);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}

export function hms(rfc: string): string {
  const d = new Date(rfc);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** "12 nov" */
export function shortDate(rfc: string): string {
  const d = new Date(rfc);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString("pt-BR", { day: "numeric", month: "short" }).replace(".", "");
}

/** "há 3 min" */
export function ago(rfc: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - new Date(rfc).getTime()) / 1000));
  if (s < 60) return `há ${s}s`;
  if (s < 3600) return `há ${Math.floor(s / 60)} min`;
  if (s < 86400) return `há ${Math.floor(s / 3600)} h`;
  return `há ${Math.floor(s / 86400)} d`;
}

export function duration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}min ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}min`;
}

export const EFFORT_LABEL: Record<string, string> = {
  low: "baixo",
  medium: "médio",
  high: "alto",
  xhigh: "muito alto",
  max: "máximo",
};

export const EFFORT_LEVEL: Record<string, number> = { low: 1, medium: 2, high: 3, xhigh: 4, max: 5 };

/** "claude-opus-5-5" -> "Opus 5.5"; "claude-opus-5" -> "Opus 5". */
export function modelLabel(m: string): string {
  const r = m.match(/(opus|sonnet|haiku|fable)-(\d+)(?:-(\d))?(?:|-|$)/);
  if (!r) return m;
  return `${r[1]![0]!.toUpperCase()}${r[1]!.slice(1)} ${r[2]}${r[3] ? `.${r[3]}` : ""}`;
}
