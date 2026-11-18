import React from "react";

/**
 * Texto com realce: trechos entre *asteriscos* saem na cor de acento.
 * Ex: "disciplina *todo dia*" -> "todo dia" em vermelho.
 */
export const Highlighted: React.FC<{
  text: string;
  accent: string;
  upper: boolean;
}> = ({ text, accent, upper }) => {
  const parts = text.split(/(\*[^*]+\*)/g).filter(Boolean);
  return (
    <>
      {parts.map((p, i) => {
        const hit = p.startsWith("*") && p.endsWith("*") && p.length > 2;
        const body = hit ? p.slice(1, -1) : p;
        const shown = upper ? body.toUpperCase() : body;
        return hit ? (
          <span key={i} style={{ color: accent }}>
            {shown}
          </span>
        ) : (
          <span key={i}>{shown}</span>
        );
      })}
    </>
  );
};

/** Quantos caracteres visiveis (ignora os marcadores de realce). */
export const plainLength = (text: string) => text.replace(/\*/g, "").length;

/** Corta preservando os marcadores — usado pelo efeito typewriter. */
export const sliceMarked = (text: string, n: number) => {
  let out = "";
  let count = 0;
  for (const ch of text) {
    if (ch === "*") {
      out += ch;
      continue;
    }
    if (count >= n) break;
    out += ch;
    count++;
  }
  // marcador aberto sem conteudo ainda: nao mostra nada
  out = out.replace(/\*+$/, "");
  const stars = (out.match(/\*/g) ?? []).length;
  return stars % 2 === 1 ? out + "*" : out;
};

/** Revela palavra a palavra, acumulando (legenda de dialogo dos edits). */
export const revealWords = (text: string, progress: number): string => {
  const palavras = text.split(/\s+/).filter(Boolean);
  if (palavras.length === 0) return text;
  const n = Math.max(1, Math.ceil(progress * palavras.length));
  const parcial = palavras.slice(0, n).join(" ");
  const estrelas = (parcial.match(/\*/g) ?? []).length;
  return estrelas % 2 === 1 ? parcial + "*" : parcial;
};
