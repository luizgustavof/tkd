import React from "react";
import { AbsoluteFill, random, useCurrentFrame, interpolate } from "remotion";
import { ease, type EasingName } from "./core";

/**
 * Grade por SVG: curva por canal (feComponentTransfer table) + saturacao.
 * `curve` sao os valores de saida para entradas igualmente espacadas 0..1.
 */
export type GradeSpec = {
  r: number[];
  g: number[];
  b: number[];
  saturate?: number;
  /** nitidez: 0 desliga, ~0.6 = unsharp leve */
  sharpen?: number;
  /** ganho por canal aplicado DEPOIS da saturacao (nao e amplificado pela gama) */
  tint?: [number, number, number];
};

/**
 * Gera uma GradeSpec de "levels": entrada [black,white] (0-255) -> [0,1], gama nos medios,
 * tint por canal (multiplicador e offset) — calibrado pelos percentis do plano de origem.
 * Ex.: black = p5 da fonte, white = p97 da fonte => ~3% de altas estouradas, como nos edits.
 */
export const levels = (o: {
  black: number;
  white: number;
  gamma?: number;
  saturate?: number;
  sharpen?: number;
  tint?: [number, number, number];
  steps?: number;
}): GradeSpec => {
  const n = o.steps ?? 17;
  const curve = Array.from({ length: n }, (_, i) => {
    const x = (i / (n - 1)) * 255;
    const v = Math.min(1, Math.max(0, (x - o.black) / Math.max(1, o.white - o.black)));
    return +Math.pow(v, o.gamma ?? 1).toFixed(4);
  });
  return { r: curve, g: curve, b: curve, saturate: o.saturate, sharpen: o.sharpen, tint: o.tint };
};

export const GradeDefs: React.FC<{ id: string; spec: GradeSpec }> = ({ id, spec }) => {
  const k = spec.sharpen ?? 0;
  return (
    <svg width="0" height="0" style={{ position: "absolute" }}>
      <filter id={id} colorInterpolationFilters="sRGB" x="0" y="0" width="100%" height="100%">
        {k > 0 ? (
          <feConvolveMatrix
            order="3"
            kernelMatrix={`0 ${-k} 0 ${-k} ${1 + 4 * k} ${-k} 0 ${-k} 0`}
            preserveAlpha="true"
            result="sharp"
          />
        ) : null}
        <feComponentTransfer>
          <feFuncR type="table" tableValues={spec.r.join(" ")} />
          <feFuncG type="table" tableValues={spec.g.join(" ")} />
          <feFuncB type="table" tableValues={spec.b.join(" ")} />
        </feComponentTransfer>
        <feColorMatrix type="saturate" values={String(spec.saturate ?? 1)} />
        {spec.tint ? (
          <feComponentTransfer>
            <feFuncR type="linear" slope={spec.tint[0]} />
            <feFuncG type="linear" slope={spec.tint[1]} />
            <feFuncB type="linear" slope={spec.tint[2]} />
          </feComponentTransfer>
        ) : null}
      </filter>
    </svg>
  );
};

/** Aplica a grade a tudo que estiver dentro. */
export const Graded: React.FC<{ id: string; spec: GradeSpec; children: React.ReactNode }> = ({ id, spec, children }) => (
  <AbsoluteFill>
    <GradeDefs id={id} spec={spec} />
    <AbsoluteFill style={{ filter: `url(#${id})` }}>{children}</AbsoluteFill>
  </AbsoluteFill>
);

/** Grao monocromatico re-semeado por frame. blend overlay escurece/clareia sem lavar pretos. */
export const FilmGrain: React.FC<{ opacity?: number; size?: number; blend?: React.CSSProperties["mixBlendMode"] }> = ({
  opacity = 0.2,
  size = 0.9,
  blend = "overlay",
}) => {
  const frame = useCurrentFrame();
  const seed = Math.floor(random(`fg-${frame}`) * 1000);
  const id = `fgrain-${frame}`;
  return (
    <AbsoluteFill style={{ opacity, mixBlendMode: blend, pointerEvents: "none" }}>
      <svg width="100%" height="100%">
        <filter id={id}>
          <feTurbulence type="fractalNoise" baseFrequency={size} numOctaves={2} seed={seed} />
          <feColorMatrix type="saturate" values="0" />
        </filter>
        <rect width="100%" height="100%" filter={`url(#${id})`} />
      </svg>
    </AbsoluteFill>
  );
};

/**
 * Efeitos de frame unico sobre tudo que estiver dentro:
 * invert (flash negativo), mono (P&B alto contraste), white (branco puro).
 * `frames` sao absolutos em relacao ao pai.
 */
export type FrameFxKind = "invert" | "mono" | "white";
export const FrameFx: React.FC<{ fx: Record<number, FrameFxKind>; children: React.ReactNode }> = ({ fx, children }) => {
  const frame = useCurrentFrame();
  const kind = fx[frame];
  const filter =
    kind === "invert"
      ? "invert(1) saturate(0.35)"
      : kind === "mono"
        ? "grayscale(1) contrast(2.4) brightness(1.15)"
        : undefined;
  return (
    <AbsoluteFill style={{ filter }}>
      {children}
      {kind === "white" ? <AbsoluteFill style={{ backgroundColor: "#fefefe" }} /> : null}
    </AbsoluteFill>
  );
};

/**
 * Flare/bloom de luz que cresce (ex.: farol entrando no plano).
 * keys: [frame, intensidade 0..1]; fontes em fracao do quadro.
 */
export const LightBloom: React.FC<{
  keys: [number, number][];
  sources: { x: number; y: number; r: number }[];
  color?: string;
  easing?: EasingName;
}> = ({ keys, sources, color = "rgba(235,242,255,1)", easing = "inOut" }) => {
  const frame = useCurrentFrame();
  const k = interpolate(
    frame,
    keys.map((p) => p[0]),
    keys.map((p) => p[1]),
    { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: ease(easing) }
  );
  if (k <= 0.001) return null;
  return (
    <AbsoluteFill style={{ mixBlendMode: "screen", pointerEvents: "none", opacity: k }}>
      {sources.map((s, i) => (
        <AbsoluteFill
          key={i}
          style={{
            background: `radial-gradient(circle at ${s.x * 100}% ${s.y * 100}%, ${color} 0%, ${color.replace(/,\s*1\)$/, ",0.55)")} ${s.r * 25}%, transparent ${s.r * 100}%)`,
          }}
        />
      ))}
    </AbsoluteFill>
  );
};

export const Vignette: React.FC<{ strength?: number; inner?: number }> = ({ strength = 0.4, inner = 40 }) => (
  <AbsoluteFill
    style={{
      background: `radial-gradient(circle at 50% 48%, transparent ${inner}%, rgba(0,0,0,${strength}) 100%)`,
      pointerEvents: "none",
    }}
  />
);

/** Marca d'agua fixa no rodape. */
export const Watermark: React.FC<{
  text: string;
  fontFamily: string;
  sizePx?: number;
  y?: number;
  color?: string;
  opacity?: number;
  letterSpacing?: string;
}> = ({ text, fontFamily, sizePx = 22, y = 0.974, color = "#bdbdbd", opacity = 0.6, letterSpacing = "0.04em" }) => (
  <AbsoluteFill style={{ pointerEvents: "none" }}>
    <div
      style={{
        position: "absolute",
        left: "50%",
        top: `${y * 100}%`,
        transform: "translate(-50%, -50%)",
        fontFamily,
        fontWeight: 700,
        fontSize: sizePx,
        letterSpacing,
        color,
        opacity,
        whiteSpace: "nowrap",
      }}
    >
      {text}
    </div>
  </AbsoluteFill>
);
