import type { CaptionStep, FrameFxKind } from "../motionlib";

/** Plano de recriacao: gerado por `tkd recreate` (agente + autograde + voz) e passado como --props. */
export type RecreatePlan = {
  id: string;
  fps: number;
  width: number;
  height: number;
  durationInFrames: number;
  shots: {
    from: number;
    to: number;
    src: string;
    startAt?: number;
    rate?: number;
    freeze?: boolean;
    camera?: { from?: { s?: number; x?: number; y?: number }; to?: { s?: number; x?: number; y?: number }; easing?: "linear" | "inOut" | "in" | "out" };
    jitter?: { x?: number; y?: number; freq?: number };
    overscan?: number;
    objectPosition?: string;
    grade?: { black: number; white: number; gamma: number; saturate?: number; tint?: [number, number, number]; sharpen?: number };
    transitionIn?: { kind: "cut" | "dissolve"; frames: number };
    ref_shot?: number;
    /** audio do proprio clipe (dialogo real); 0/ausente = mudo */
    volume?: number;
    /** efeito do plano inteiro depois do grade */
    fx?: "none" | "invert" | "threshold" | "mono";
    /** parallax 2.5D: troca o video por duas camadas (scripts/parallax_layers.py) */
    layers?: { bg: string; fg: string; mode?: "pan" | "dolly"; amount?: number; dir?: number };
    note?: string;
  }[];
  captions?: { seed?: string; steps: CaptionStep[]; end: number }[];
  captionStyle: {
    font?: string;
    weight?: number;
    italic?: boolean;
    sizePx: number;
    x?: number;
    y?: number;
    color?: string;
    strokePx?: number;
    strokeColor?: string;
    shadow?: string;
    letterSpacing?: string;
    upper?: boolean;
    opacity?: number;
    maxWidth?: number;
    lineHeight?: number;
    twitch?: { blank?: number; back?: number } | null;
    stretch?: { sx?: number; sy?: number; blurPx?: number } | null;
    pop?: { scale?: number; frames?: number } | null;
  };
  /** eventos de movimento (punch/pulse/shake/rgb/flash/blur) — scripts/automotion.py gera a partir dos cortes e da musica */
  motion?: { f: number; kind: "punch" | "pulse" | "shake" | "rgb" | "flash" | "blur" | "zoomblur" | "warp" | "glow" | "rays";
    amount: number; dur: number; x?: number; y?: number }[];
  title?: {
    steps: CaptionStep[];
    end: number;
    font?: string;
    capHeight?: number;
    scaleX?: number;
    scaleY?: number;
    y?: number;
    blend?: string;
    glowPx?: number;
    glowOpacity?: number;
    fringePx?: number;
    color?: string;
  } | null;
  frameFx?: Record<string, FrameFxKind>;
  /** falas reais tocando por cima dos planos (audio de outro clipe); from/to/startAt preenchidos por clip_voice.py */
  voiceClips?: { src: string; line: number; at: number; volume?: number; from?: number; to?: number; startAt?: number;
    translation?: string }[];
  blooms?: { from: number; to: number; keys: [number, number][]; sources: { x: number; y: number; r: number }[] }[];
  watermark?: { text: string; from?: number; font?: string; sizePx?: number; y?: number; color?: string; opacity?: number } | null;
  look?: { grain?: number; grainSize?: number; vignette?: number; vignetteInner?: number; glow?: number;
    /** Deep Glow do frame inteiro (bright pass + 3 blurs somados), reforcado por eventos `glow` */
    deepGlow?: { amount: number; threshold?: number; radius?: number } };
  endCard?: { from: number; text: string; font?: string; sizePx?: number; color?: string; background?: string } | null;
  audio?: { music?: string; musicStart?: number; musicVolume?: number; voice?: string; voiceVolume?: number;
    /** ganho da musica durante fala real do clipe (padrao 0.35) */
    duck?: number };
};

export const emptyPlan: RecreatePlan = {
  id: "preview",
  fps: 30,
  width: 1080,
  height: 1080,
  durationInFrames: 60,
  shots: [],
  captionStyle: { sizePx: 44 },
};
