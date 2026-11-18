import React from "react";
import { AbsoluteFill, useCurrentFrame, useVideoConfig } from "remotion";
import { glitchAt } from "./core";

/**
 * Um passo da legenda cumulativa: no frame `f` (relativo a Sequence) o texto passa a ser `text`.
 * `stretch` = frame de entrada com a palavra esticada (wide stretch de 1 frame).
 */
export type CaptionStep = { f: number; text: string; stretch?: boolean; color?: string };

export type CaptionStyle = {
  fontFamily: string;
  weight?: number;
  italic?: boolean;
  /** altura da fonte em px */
  sizePx: number;
  /** centro vertical como fracao da altura (0.5 = meio) */
  y?: number;
  x?: number;
  color?: string;
  /** contorno fino (px) */
  strokePx?: number;
  strokeColor?: string;
  shadow?: string;
  letterSpacing?: string;
  upper?: boolean;
  /** largura maxima (fracao do quadro); com isso o texto quebra linha e centraliza */
  maxWidth?: number;
  lineHeight?: number;
};

export type CaptionTrackProps = {
  steps: CaptionStep[];
  /** frame (relativo) em que a frase some (corte seco) */
  end: number;
  style: CaptionStyle;
  /** gagueira: some (blank) ou volta ao passo anterior (back) em frames aleatorios */
  twitch?: { seed: string; blank?: number; back?: number };
  /** geometria do frame esticado */
  stretch?: { sx?: number; sy?: number; blurPx?: number };
  opacity?: number;
  /** entrada de cada passo: escala `scale` -> 1 em `frames` (0/ausente = corte seco) */
  pop?: { scale?: number; frames?: number };
};

let uid = 0;

/** Legenda palavra-a-palavra/silaba-a-silaba com stretch de entrada e twitch. */
export const CaptionTrack: React.FC<CaptionTrackProps> = ({ steps, end, style, twitch, stretch, opacity = 1, pop }) => {
  const frame = useCurrentFrame();
  const { height, width } = useVideoConfig();
  const [filterId] = React.useState(() => `hblur-${uid++}`);
  if (frame >= end) return null;

  let idx = -1;
  for (let i = 0; i < steps.length; i++) if (steps[i].f <= frame) idx = i;
  if (idx < 0) return null;

  let step = steps[idx];
  const isStretch = !!step.stretch && frame === step.f;
  if (twitch && !isStretch) {
    const pb = twitch.blank ?? 0.16;
    const pk = twitch.back ?? 0.08;
    if (glitchAt(`${twitch.seed}-b`, frame, pb)) return null;
    if (idx > 0 && glitchAt(`${twitch.seed}-k`, frame, pk)) {
      // volta ao texto do passo anterior (pulando o frame esticado)
      step = { ...steps[idx - 1], stretch: false };
    }
  }

  const sx = stretch?.sx ?? 2.8;
  const sy = stretch?.sy ?? 0.85;
  const blur = stretch?.blurPx ?? 5;
  const text = style.upper === false ? step.text : step.text.toUpperCase();
  const stretched = isStretch && step.stretch;
  const popF = pop?.frames ?? 0;
  const since = frame - step.f;
  const popScale = !stretched && popF > 0 && since >= 0 && since < popF
    ? 1 + ((pop?.scale ?? 1.15) - 1) * (1 - since / popF) ** 2
    : 1;

  return (
    <AbsoluteFill style={{ pointerEvents: "none", opacity }}>
      <svg width="0" height="0" style={{ position: "absolute" }}>
        <filter id={filterId} x="-50%" y="-50%" width="200%" height="200%">
          <feGaussianBlur stdDeviation={`${blur} 0`} />
        </filter>
      </svg>
      <div
        style={{
          position: "absolute",
          left: (style.x ?? 0.5) * width,
          top: (style.y ?? 0.5) * height,
          transform: `translate(-50%, -50%)${stretched ? ` scale(${sx}, ${sy})` : popScale !== 1 ? ` scale(${popScale.toFixed(4)})` : ""}`,
          whiteSpace: style.maxWidth ? "pre-line" : "pre",
          width: style.maxWidth ? style.maxWidth * width : undefined,
          textAlign: "center",
          fontFamily: style.fontFamily,
          fontWeight: style.weight ?? 600,
          fontStyle: style.italic ? "italic" : "normal",
          fontSize: style.sizePx,
          lineHeight: style.lineHeight ?? 1,
          letterSpacing: style.letterSpacing,
          color: step.color ?? style.color ?? "#f8f9fb",
          WebkitTextStroke: style.strokePx ? `${style.strokePx}px ${style.strokeColor ?? "#1e2022"}` : undefined,
          paintOrder: "stroke fill",
          textShadow: style.shadow ?? "0 1px 2px rgba(0,0,0,.85), 0 2px 8px rgba(0,0,0,.55)",
          filter: stretched ? `url(#${filterId})` : undefined,
        }}
      >
        {text}
      </div>
    </AbsoluteFill>
  );
};
