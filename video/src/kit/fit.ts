import { measureText } from "@remotion/layout-utils";

type Args = {
  text: string;
  fontFamily: string;
  fontWeight: number;
  /** tamanho que o roteiro pediu, em px */
  desired: number;
  boxWidth: number;
  boxHeight: number;
  lineHeight: number;
};

/**
 * Corpo de letra que realmente cabe na caixa.
 * Mede o texto de verdade em vez de chutar por contagem de caracteres:
 * nenhuma palavra pode estourar a largura, e o bloco inteiro precisa
 * caber na altura depois de quebrar em linhas.
 */
export const fitFontSize = ({
  text,
  fontFamily,
  fontWeight,
  desired,
  boxWidth,
  boxHeight,
  lineHeight,
}: Args): number => {
  const limpo = text.replace(/\*/g, "").trim();
  if (!limpo) return desired;

  const medir = (t: string, fontSize: number) =>
    measureText({ text: t, fontFamily, fontWeight, fontSize, validateFontIsLoaded: false });

  const BASE = 100;
  const palavras = limpo.split(/\s+/);

  // 1) nenhuma palavra isolada pode passar da largura
  const maiorPalavra = palavras.reduce(
    (max, p) => Math.max(max, medir(p, BASE).width),
    1
  );
  const porPalavra = (boxWidth / maiorPalavra) * BASE;

  // 2) o bloco quebrado em linhas tem de caber na altura
  const larguraTotal = medir(limpo, BASE).width;
  let size = Math.min(desired, porPalavra);
  for (let i = 0; i < 12; i++) {
    const largura = (larguraTotal * size) / BASE;
    const linhas = Math.max(1, Math.ceil(largura / boxWidth));
    const altura = linhas * size * lineHeight;
    if (altura <= boxHeight) break;
    size *= Math.sqrt(boxHeight / altura);
  }
  return Math.max(8, Math.floor(size));
};

/**
 * Cartela de impacto: o texto CRESCE ate ocupar a largura da caixa
 * (a legenda apenas encolhe; aqui o objetivo e preencher).
 */
export const fillFontSize = ({
  text,
  fontFamily,
  fontWeight,
  boxWidth,
  boxHeight,
  lineHeight,
}: Omit<Args, "desired">): number => {
  const limpo = text.replace(/\*/g, "").trim();
  if (!limpo) return 48;
  const BASE = 100;
  const medir = (t: string) =>
    measureText({ text: t, fontFamily, fontWeight, fontSize: BASE, validateFontIsLoaded: false })
      .width;

  const palavras = limpo.split(/\s+/);
  // a palavra mais larga define o teto: nada pode vazar na horizontal
  const maior = palavras.reduce((m, p) => Math.max(m, medir(p)), 1);
  let size = (boxWidth / maior) * BASE;

  // com mais de uma palavra, cada uma vira uma linha — tem de caber na altura
  const linhas = palavras.length;
  const altura = linhas * size * lineHeight;
  if (altura > boxHeight) size *= boxHeight / altura;

  return Math.max(12, Math.floor(size));
};
