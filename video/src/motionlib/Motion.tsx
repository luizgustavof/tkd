import React from "react";
import { AbsoluteFill, Easing, useCurrentFrame, useVideoConfig } from "remotion";
import { coverScale, smoothNoise } from "./core";

/**
 * Eventos de movimento por frame (absolutos na composicao):
 *  - punch : zoom de impacto no corte — comeca em `amount` (ex. 1.12) e volta a 1 em `dur` frames (ease out)
 *  - pulse : batida — sobe para `amount` (ex. 1.03) e volta, curva em sino
 *  - shake : tremor que decai — `amount` = px de amplitude
 *  - rgb   : aberracao cromatica que decai — `amount` = px de deslocamento R/B
 *  - flash : clarao branco que decai — `amount` = opacidade inicial (0..1)
 *  - blur  : borrão direcional de impacto (zoom-blur aproximado) — `amount` = px
 *  - zoomblur : borrão radial (CC Radial Blur) — `amount` = zoom extra da ultima copia (ex. 0.2)
 *  - warp  : distorcao por ruido (Turbulent Displace), curva em sino — `amount` = px de deslocamento
 *  - glow  : reforco do Deep Glow (precisa de `glow` ligado) — `amount` = intensidade extra
 *  - rays  : raios de luz das partes claras (CC Light Rays), curva em sino — `amount` = intensidade, `x`/`y` = origem 0..1
 */
export type MotionKind = "punch" | "pulse" | "shake" | "rgb" | "flash" | "blur" | "zoomblur" | "warp" | "glow" | "rays";
export type MotionEvent = { f: number; kind: MotionKind; amount: number; dur: number; x?: number; y?: number };
/** Deep Glow do frame inteiro: so o que passa de `threshold` (0..1) brilha, em 3 raios somados */
export type DeepGlow = { amount: number; threshold?: number; radius?: number };

const out = Easing.out(Easing.cubic);

/** intensidade 0..1 do evento no frame atual (0 fora da janela) */
const level = (e: MotionEvent, frame: number) => {
  const t = (frame - e.f) / Math.max(1, e.dur);
  if (t < 0 || t > 1) return 0;
  if (e.kind === "pulse" || e.kind === "rays" || e.kind === "warp") return Math.sin(Math.PI * t); // sobe e desce
  return 1 - out(t); // decai
};

let uid = 0;

/** Aplica os eventos ao conteudo (footage). Texto fica fora para continuar legivel. */
export const MotionFx: React.FC<{ events: MotionEvent[]; seed?: string; glow?: DeepGlow; children: React.ReactNode }> = ({
  events,
  seed = "mfx",
  glow,
  children,
}) => {
  const frame = useCurrentFrame();
  const { width, height } = useVideoConfig();
  const [fid] = React.useState(() => `rgbsplit-${uid++}`);
  const [gid] = React.useState(() => `fxg-${uid++}`);
  let scale = 1;
  let x = 0;
  let y = 0;
  let rgb = 0;
  let flash = 0;
  let blur = 0;
  let zoom = 0;
  let warp = 0;
  let glowBoost = 0;
  let rays = 0;
  let rx = 0.5;
  let ry = 0.35;
  for (const e of events) {
    if (frame < e.f || frame > e.f + e.dur) continue;
    const k = level(e, frame);
    if (k <= 0) continue;
    if (e.kind === "punch" || e.kind === "pulse") scale *= 1 + (e.amount - 1) * k;
    if (e.kind === "shake") {
      x += smoothNoise(`${seed}x${e.f}`, frame / 30, 9) * e.amount * k;
      y += smoothNoise(`${seed}y${e.f}`, frame / 30, 9) * e.amount * k;
    }
    if (e.kind === "rgb") rgb = Math.max(rgb, e.amount * k);
    if (e.kind === "flash") flash = Math.max(flash, e.amount * k);
    if (e.kind === "blur") blur = Math.max(blur, e.amount * k);
    if (e.kind === "zoomblur") zoom = Math.max(zoom, e.amount * k);
    if (e.kind === "warp") warp = Math.max(warp, e.amount * k);
    if (e.kind === "glow") glowBoost = Math.max(glowBoost, e.amount * k);
    if (e.kind === "rays" && e.amount * k > rays) {
      rays = e.amount * k;
      rx = e.x ?? 0.5;
      ry = e.y ?? 0.35;
    }
  }
  const glowAmt = glow ? glow.amount + glowBoost : 0;
  const thr = glow?.threshold ?? 0.6;
  const rad = (glow?.radius ?? 1) * (height / 1080);
  // shake desloca a camada inteira: sem zoom junto, o tremor mostra a borda preta
  if (x || y) scale = Math.max(scale, coverScale(width, height, x, y));
  const filters = [
    rgb > 0.3 ? `url(#${fid})` : "",
    warp > 0.5 ? `url(#${gid}-warp)` : "",
    blur > 0.3 ? `blur(${blur.toFixed(2)}px)` : "",
  ].join(" ").trim();
  const transform = scale !== 1 || x || y ? `translate(${x.toFixed(2)}px, ${y.toFixed(2)}px) scale(${scale.toFixed(4)})` : undefined;
  // copias do proprio conteudo: zoom radial (media corrente de copias escaladas), bright pass do glow e dos raios
  const ZB = 7;
  const RAYS = 9;
  const bpSlope = 1 / Math.max(0.05, 1 - thr);
  const bp = (
    <feComponentTransfer in="SourceGraphic" result="bp">
      <feFuncR type="linear" slope={bpSlope} intercept={-thr * bpSlope} />
      <feFuncG type="linear" slope={bpSlope} intercept={-thr * bpSlope} />
      <feFuncB type="linear" slope={bpSlope} intercept={-thr * bpSlope} />
    </feComponentTransfer>
  );
  return (
    <AbsoluteFill>
      <svg width="0" height="0" style={{ position: "absolute" }}>
        {rgb > 0.3 ? (
          <filter id={fid} x="-5%" y="-5%" width="110%" height="110%" colorInterpolationFilters="sRGB">
            <feColorMatrix in="SourceGraphic" type="matrix" values="1 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0" result="r" />
            <feOffset in="r" dx={rgb} dy={0} result="ro" />
            <feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0" result="g" />
            <feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 1 0 0  0 0 0 1 0" result="b" />
            <feOffset in="b" dx={-rgb} dy={0} result="bo" />
            <feBlend in="ro" in2="g" mode="screen" result="rg" />
            <feBlend in="rg" in2="bo" mode="screen" />
          </filter>
        ) : null}
        {warp > 0.5 ? (
          <filter id={`${gid}-warp`} x="-5%" y="-5%" width="110%" height="110%">
            <feTurbulence type="fractalNoise" baseFrequency="0.004 0.012" numOctaves={2} seed={Math.floor(frame / 2)} result="n" />
            <feDisplacementMap in="SourceGraphic" in2="n" scale={warp} xChannelSelector="R" yChannelSelector="G" />
          </filter>
        ) : null}
        {glowAmt > 0.01 ? (
          <filter id={`${gid}-glow`} x="-10%" y="-10%" width="120%" height="120%" colorInterpolationFilters="sRGB">
            {bp}
            <feGaussianBlur in="bp" stdDeviation={6 * rad} result="b1" />
            <feGaussianBlur in="bp" stdDeviation={20 * rad} result="b2" />
            <feGaussianBlur in="bp" stdDeviation={55 * rad} result="b3" />
            <feComposite in="b1" in2="b2" operator="arithmetic" k2={0.9} k3={0.8} result="b12" />
            <feComposite in="b12" in2="b3" operator="arithmetic" k2={1} k3={0.75} />
          </filter>
        ) : null}
        {rays > 0.01 ? (
          <filter id={`${gid}-rays`} colorInterpolationFilters="sRGB">
            {bp}
            <feGaussianBlur in="bp" stdDeviation={2.5 * rad} />
          </filter>
        ) : null}
      </svg>
      <AbsoluteFill style={{ transform, filter: filters || undefined }}>
        <AbsoluteFill>{children}</AbsoluteFill>
        {zoom > 0.005
          ? Array.from({ length: ZB - 1 }, (_, i) => (
              <AbsoluteFill key={`zb${i}`} style={{ opacity: 1 / (i + 2), transform: `scale(${(1 + (zoom * (i + 1)) / (ZB - 1)).toFixed(4)})` }}>
                {children}
              </AbsoluteFill>
            ))
          : null}
        {glowAmt > 0.01 ? (
          <AbsoluteFill style={{ mixBlendMode: "screen", opacity: Math.min(1, glowAmt), filter: `url(#${gid}-glow)` }}>{children}</AbsoluteFill>
        ) : null}
        {rays > 0.01 ? (
          <AbsoluteFill style={{ mixBlendMode: "screen", opacity: Math.min(1, rays), isolation: "isolate" }}>
            {Array.from({ length: RAYS }, (_, i) => (
              <AbsoluteFill
                key={`ry${i}`}
                style={{
                  // cada copia passa pelo bright pass e soma em screen: rastro que decai a partir da fonte
                  filter: `url(#${gid}-rays)`,
                  mixBlendMode: "screen",
                  opacity: 0.85 * (1 - i / RAYS),
                  transformOrigin: `${(rx * 100).toFixed(1)}% ${(ry * 100).toFixed(1)}%`,
                  transform: `scale(${(1 + (0.9 * i) / (RAYS - 1)).toFixed(4)})`,
                }}
              >
                {children}
              </AbsoluteFill>
            ))}
          </AbsoluteFill>
        ) : null}
      </AbsoluteFill>
      {flash > 0.01 ? <AbsoluteFill style={{ backgroundColor: "#fff", opacity: flash, pointerEvents: "none" }} /> : null}
    </AbsoluteFill>
  );
};
