// Idioma da interface: pt-BR (texto-fonte), English e العربية (RTL).
// A chave de cada texto e o proprio texto em pt-BR; `t()` so aceita chaves de messages.ts (o tsc acusa
// texto sem traducao). `tx()` traduz texto que vem pronto do backend, por igualdade ou por padrao.
import { useSyncExternalStore } from "react";
import { BACKEND, M, type MsgKey, type Tr } from "./messages";

export type Lang = "pt" | "en" | "ar";
export const LANGS: { value: Lang; label: string; hint: string }[] = [
  { value: "pt", label: "Português (Brasil)", hint: "Portuguese" },
  { value: "en", label: "English", hint: "Inglês" },
  { value: "ar", label: "العربية", hint: "Arabic · RTL" },
];
const KEY = "tkd.lang";
const TAG: Record<Lang, string> = { pt: "pt-BR", en: "en", ar: "ar" };
const LOCALE: Record<Lang, string> = { pt: "pt-BR", en: "en-US", ar: "ar-u-nu-latn" };

function initial(): Lang {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "pt" || v === "en" || v === "ar") return v;
  } catch {
    // sem storage: segue o sistema
  }
  const n = (navigator.language || "pt").toLowerCase();
  return n.startsWith("ar") ? "ar" : n.startsWith("en") ? "en" : "pt";
}

let lang: Lang = initial();
const ls = new Set<() => void>();

function apply() {
  const el = document.documentElement;
  el.lang = TAG[lang];
  el.dir = lang === "ar" ? "rtl" : "ltr";
}
apply();

export function getLang(): Lang {
  return lang;
}
export const isRtl = () => lang === "ar";
export const locale = () => LOCALE[lang];

export function setLang(l: Lang) {
  if (l === lang) return;
  lang = l;
  try {
    localStorage.setItem(KEY, l);
  } catch {
    // so nao lembra na proxima abertura
  }
  apply();
  for (const f of ls) f();
}

/** Re-renderiza quando o idioma muda (a raiz do app usa como `key`). */
export function useLang(): Lang {
  return useSyncExternalStore(
    (f) => (ls.add(f), () => ls.delete(f)),
    () => lang,
  );
}

type Vars = Record<string, string | number>;
const fill = (s: string, v?: Vars) => (v ? s.replace(/\{(\w+)\}/g, (_, k: string) => String(v[k] ?? `{${k}}`)) : s);

/** Texto da interface. */
export function t(k: MsgKey, vars?: Vars): string {
  return fill(lang === "pt" ? k : (M[k] as Tr)[lang], vars);
}

// padroes do backend ("Postado às {hm}"): os mais especificos (mais texto fixo) primeiro
const PATTERNS = Object.entries(BACKEND)
  .filter(([k]) => k.includes("{"))
  .map(([k, tr]) => {
    const names: string[] = [];
    const src = k.replace(/[.*+?^$()|[\]\\]/g, "\\$&").replace(/\\?\{(\w+)\\?\}/g, (_, n: string) => (names.push(n), "(.+?)"));
    return { re: new RegExp(`^${src}$`), names, tr, fixed: k.replace(/\{\w+\}/g, "").length };
  })
  .sort((a, b) => b.fixed - a.fixed);

/** Texto pronto vindo do backend (rotulos, etapas, horarios): traduz o que conhece, o resto passa igual. */
export function tx(s: string, depth = 0): string {
  if (lang === "pt" || !s) return s;
  const hit = BACKEND[s] ?? (M as Record<string, Tr>)[s];
  if (hit) return hit[lang];
  if (depth > 4) return s;
  for (const p of PATTERNS) {
    const m = s.match(p.re);
    if (!m) continue;
    const v: Vars = {};
    p.names.forEach((n, i) => (v[n] = tx(m[i + 1]!, depth + 1)));
    return fill(p.tr[lang], v);
  }
  return s;
}
