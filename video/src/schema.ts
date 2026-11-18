import { z } from "zod";

export const paletteSchema = z.object({
  bg: z.string().default("#000000"),
  fg: z.string().default("#ffffff"),
  accent: z.string().default("#ff2e63"),
});

export const watermarkSchema = z.object({
  text: z.string().default(""),
  sizeVh: z.number().default(2.2),
  opacity: z.number().min(0).max(1).default(0.55),
  color: z.string().default("#bfbfbf"),
});

export const typographySchema = z.object({
  family: z.enum(["Inter", "Anton", "BebasNeue", "Poppins", "PlayfairDisplay"]).default("Inter"),
  weight: z.number().default(800),
  case: z.enum(["upper", "normal"]).default("upper"),
  sizeVh: z.number().default(7),
  style: z.enum(["normal", "italic"]).default("normal"),
});

export const treatmentSchema = z.object({
  grain: z.number().min(0).max(1).default(0.12),
  vignette: z.number().min(0).max(1).default(0.3),
  zoom: z.enum(["none", "slow-in", "slow-out", "punch"]).default("slow-in"),
  glitch: z.boolean().default(false),
  /** esmaga os pretos e estoura os brancos (curva em S) */
  crush: z.number().min(0).max(1).default(0.4),
  /** brilho difuso nos altos */
  bloom: z.number().min(0).max(1).default(0.3),
  blurBg: z.number().min(0).max(40).default(0),
});

export const sceneSchema = z.object({
  startS: z.number(),
  durationS: z.number(),
  kind: z.enum(["text", "quote", "media", "title"]).default("text"),
  text: z.string().default(""),
  sub: z.string().default(""),
  /** corpo de letra desta cena (vh); sem isso usa o da tipografia do canal */
  sizeVh: z.number().min(1).max(45).optional(),
  /** caminho de arquivo relativo a video/public/, se houver */
  media: z.string().optional(),
  /** segundo de entrada dentro do clipe */
  mediaStart: z.number().min(0).optional(),
  /** volume do audio do proprio clipe (fala do personagem) */
  volume: z.number().min(0).max(1).default(0),
  effect: z
    .enum(["none", "shake", "flash", "slide", "typewriter", "words", "invert"])
    .default("none"),
  /** cartela gigante: o texto ocupa a largura toda */
  fill: z.boolean().optional(),
});

export const specSchema = z.object({
  id: z.string(),
  channel: z.string(),
  title: z.string().default(""),
  durationS: z.number().min(5).max(40).default(15),
  /** formato do quadro; as referencias de alguns canais sao 1:1, nao 9:16 */
  aspect: z.enum(["9:16", "1:1", "4:5", "4:3"]).default("9:16"),
  fps: z.number().default(30),
  palette: paletteSchema,
  typography: typographySchema,
  treatment: treatmentSchema,
  scenes: z.array(sceneSchema).min(1),
  audio: z.string().optional(),
  /** segundo de entrada da trilha */
  audioStart: z.number().min(0).optional(),
  audioVolume: z.number().min(0).max(1).default(0.8),
  watermark: watermarkSchema.optional(),
  caption: z.string().default(""),
  hashtags: z.array(z.string()).default([]),
});

export type Spec = z.infer<typeof specSchema>;

export const defaultSpec: Spec = {
  id: "preview",
  channel: "preview",
  title: "preview",
  durationS: 12,
  aspect: "9:16",
  fps: 30,
  audioVolume: 0.8,
  palette: { bg: "#08080c", fg: "#f5f5f7", accent: "#ff2e63" },
  typography: { family: "Anton", weight: 400, case: "upper", sizeVh: 9, style: "normal" },
  treatment: { grain: 0.14, vignette: 0.35, zoom: "slow-in", glitch: false, blurBg: 0, crush: 0.4, bloom: 0.3 },
  scenes: [
    { startS: 0, durationS: 4, kind: "title", text: "hook aqui", sub: "", effect: "flash", volume: 0 },
    { startS: 4, durationS: 4, kind: "quote", text: "linha do meio", sub: "", effect: "slide", volume: 0 },
    { startS: 8, durationS: 4, kind: "text", text: "fecho", sub: "", effect: "none", volume: 0 },
  ],
  caption: "",
  hashtags: [],
};
