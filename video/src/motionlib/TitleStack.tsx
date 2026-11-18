import React from "react";
import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import type { CaptionStep } from "./CaptionTrack";

export type TitleStackProps = {
  /** cada passo troca o texto inteiro em corte seco (ex.: YOU'RE -> YOU'RE A -> SOR- -> SORCERER) */
  steps: CaptionStep[];
  end: number;
  fontFamily: string;
  weight?: number;
  /** altura das maiusculas como fracao do quadro */
  capHeight?: number;
  /** razao cap-height/em da fonte (League Gothic ~0.74) */
  capRatio?: number;
  /** estica vertical (fontes condensadas viram ultra-condensadas) */
  scaleY?: number;
  /** comprime horizontal */
  scaleX?: number;
  /** centro vertical (fracao) */
  y?: number;
  color?: string;
  /** modo de mistura com a imagem: difference inverte onde a imagem e clara */
  blend?: React.CSSProperties["mixBlendMode"];
  /** halo branco atras das letras (px de blur, 0 desliga) */
  glowPx?: number;
  glowOpacity?: number;
  letterSpacing?: string;
  /** escala de entrada no primeiro frame de cada passo (1 = corte seco) */
  punch?: number;
  /** franja cromatica: deslocamento (px) de copias ciano/vermelha atras do texto (0 desliga) */
  fringePx?: number;
  fringeColors?: [string, string];
};

/** Cartela gigante palavra a palavra, misturada com a imagem (difference + glow). */
export const TitleStack: React.FC<TitleStackProps> = ({
  steps,
  end,
  fontFamily,
  weight = 400,
  capHeight = 0.6,
  capRatio = 0.74,
  scaleY = 1.6,
  scaleX = 1,
  y = 0.5,
  color = "#ffffff",
  blend = "difference",
  glowPx = 12,
  glowOpacity = 0.55,
  letterSpacing = "0.01em",
  punch = 1,
  fringePx = 0,
  fringeColors = ["#22d3ff", "#ff2a55"],
}) => {
  const frame = useCurrentFrame();
  const { height } = useVideoConfig();
  if (frame >= end) return null;
  let idx = -1;
  for (let i = 0; i < steps.length; i++) if (steps[i].f <= frame) idx = i;
  if (idx < 0) return null;
  const step = steps[idx];
  // passo vazio = intervalo entre dois titulos
  if (!step.text) return null;
  const fontSize = (capHeight * height) / (capRatio * scaleY);
  const sc = frame === step.f ? punch : 1;

  const text: React.CSSProperties = {
    position: "absolute",
    left: "50%",
    top: `${y * 100}%`,
    transform: `translate(-50%, -50%) scale(${scaleX * sc}, ${scaleY * sc})`,
    fontFamily,
    fontWeight: weight,
    fontSize,
    lineHeight: 1,
    letterSpacing,
    whiteSpace: "nowrap",
    color,
    textShadow: fringePx
      ? `${-fringePx}px 0 0 ${fringeColors[0]}, ${fringePx}px 0 0 ${fringeColors[1]}`
      : undefined,
  };

  return (
    <>
      {glowPx > 0 ? (
        <AbsoluteFill style={{ mixBlendMode: "screen", opacity: glowOpacity, pointerEvents: "none" }}>
          <div style={{ ...text, filter: `blur(${glowPx}px)` }}>{step.text}</div>
        </AbsoluteFill>
      ) : null}
      <AbsoluteFill style={{ mixBlendMode: blend, pointerEvents: "none" }}>
        <div style={text}>{step.text}</div>
      </AbsoluteFill>
    </>
  );
};
