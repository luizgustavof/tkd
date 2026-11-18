import { Easing, interpolate, random } from "remotion";

/**
 * Contrato comum da biblioteca de motion.
 * Todo componente animado aceita estes campos; frames sao relativos a <Sequence> pai.
 */
export type EasingName =
  | "linear"
  | "inOut"
  | "in"
  | "out"
  | "outExpo"
  | "inExpo"
  | "outBack"
  | [number, number, number, number]; // cubic-bezier

export type Direction = "left" | "right" | "up" | "down" | "none";

export type AnimProps = {
  /** frames de espera antes de comecar */
  delay?: number;
  /** duracao da animacao em frames */
  durationInFrames?: number;
  /** deslocamento em px (ou fracao do quadro quando < 1 e marcado no componente) */
  x?: number;
  y?: number;
  scale?: number;
  opacity?: number;
  /** multiplicador de velocidade: 2 = duas vezes mais rapido */
  speed?: number;
  easing?: EasingName;
  direction?: Direction;
};

export const ease = (e: EasingName = "linear"): ((t: number) => number) => {
  if (Array.isArray(e)) return Easing.bezier(e[0], e[1], e[2], e[3]);
  switch (e) {
    case "inOut":
      return Easing.inOut(Easing.cubic);
    case "in":
      return Easing.in(Easing.cubic);
    case "out":
      return Easing.out(Easing.cubic);
    case "outExpo":
      return Easing.out(Easing.exp);
    case "inExpo":
      return Easing.in(Easing.exp);
    case "outBack":
      return Easing.out(Easing.back(1.4));
    default:
      return Easing.linear;
  }
};

/** progresso 0..1 de uma animacao com delay/duracao/velocidade/curva. */
export const progress = (frame: number, a: AnimProps, fallbackDur = 15) => {
  const dur = Math.max(1, (a.durationInFrames ?? fallbackDur) / (a.speed ?? 1));
  return interpolate(frame - (a.delay ?? 0), [0, dur], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: ease(a.easing),
  });
};

export const dirVector = (d: Direction = "none"): [number, number] =>
  d === "left" ? [-1, 0] : d === "right" ? [1, 0] : d === "up" ? [0, -1] : d === "down" ? [0, 1] : [0, 0];

/**
 * Ruido suave deterministico (soma de senos com fases sorteadas pela seed).
 * Retorna ~[-1,1]. freq em ciclos por segundo.
 */
/**
 * Menor escala que ainda cobre o quadro W x H quando a camada (do mesmo tamanho, origem no centro) recebe
 * `translate(x, y) scale(s) rotate(rot)`. Abaixo dela aparece o fundo preto na borda (canal de edits, 17/11: zoom 1.095
 * com pan de 80 px deixava ~30 px pretos na direita/topo). Cada canto do quadro, levado de volta para a camada
 * (u = R(-rot) * (canto - t)), precisa cair dentro dela: s >= |u.x| / (W/2) e s >= |u.y| / (H/2).
 */
export const coverScale = (W: number, H: number, x: number, y: number, rotDeg = 0): number => {
  const r = (-rotDeg * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  let need = 1;
  for (const [px, py] of [[-W / 2, -H / 2], [W / 2, -H / 2], [W / 2, H / 2], [-W / 2, H / 2]]) {
    const dx = px - x;
    const dy = py - y;
    const ux = c * dx - s * dy;
    const uy = s * dx + c * dy;
    need = Math.max(need, Math.abs(ux) / (W / 2), Math.abs(uy) / (H / 2));
  }
  return need;
};

export const smoothNoise =(seed: string, t: number, freq = 0.5) => {
  let v = 0;
  let norm = 0;
  for (let k = 0; k < 3; k++) {
    const f = freq * (1 + k * 1.7);
    const ph = random(`${seed}-ph${k}`) * Math.PI * 2;
    const amp = 1 / (k + 1);
    v += Math.sin(t * f * Math.PI * 2 + ph) * amp;
    norm += amp;
  }
  return v / norm;
};

/** true se o frame for um "glitch" (sem glitches consecutivos). */
export const glitchAt = (seed: string, frame: number, p: number) => {
  const hit = (f: number) => random(`${seed}-${f}`) < p;
  return hit(frame) && !hit(frame - 1);
};
