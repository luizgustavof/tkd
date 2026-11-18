import React from "react";
import { AbsoluteFill, Audio, interpolate, Sequence, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { loadFont as arimo } from "@remotion/google-fonts/Arimo";
import { loadFont as leagueGothic } from "@remotion/google-fonts/LeagueGothic";
import { loadFont as anton } from "@remotion/google-fonts/Anton";
import { loadFont as robotoCondensed } from "@remotion/google-fonts/RobotoCondensed";
import { loadFont as cormorant } from "@remotion/google-fonts/CormorantGaramond";
import { loadFont as inter } from "@remotion/google-fonts/Inter";
import {
  CaptionTrack,
  FilmGrain,
  FrameFx,
  Graded,
  levels,
  LightBloom,
  MotionFx,
  Parallax,
  Shot,
  TitleStack,
  Vignette,
  Watermark,
  type CaptionStep,
  type FrameFxKind,
} from "../motionlib";
import type { RecreatePlan } from "./types";

const sub = { subsets: ["latin", "latin-ext"] as ("latin" | "latin-ext")[] };
/** fontes disponiveis para planos de recriacao (nome no plano -> familia CSS) */
export const FONTS: Record<string, string> = {
  Arimo: arimo("normal", { weights: ["400", "700"], ...sub }).fontFamily,
  ArimoItalic: arimo("italic", { weights: ["400", "500", "600", "700"], ...sub }).fontFamily,
  LeagueGothic: leagueGothic("normal", { weights: ["400"], ...sub }).fontFamily,
  Anton: anton("normal", { weights: ["400"], ...sub }).fontFamily,
  RobotoCondensed: robotoCondensed("normal", { weights: ["700"], ...sub }).fontFamily,
  CormorantItalic: cormorant("italic", { weights: ["400", "500"], ...sub }).fontFamily,
  Inter: inter("normal", { weights: ["500", "600"], ...sub }).fontFamily,
};
/** efeito de plano inteiro aplicado depois do grade (Ato 2 do exp-01: planos invertidos / P&B estourado) */
const SHOT_FX: Record<string, string | undefined> = {
  none: undefined,
  invert: "invert(1) saturate(0.6)",
  threshold: "grayscale(1) contrast(6) brightness(1.1)",
  mono: "grayscale(1) contrast(2.4) brightness(1.15)",
};
const font = (n?: string) => FONTS[n ?? "Arimo"] ?? FONTS.Arimo;

/** Fade-in de dissolve no inicio do plano (o plano anterior continua por baixo). */
const DissolveIn: React.FC<{ frames: number; children: React.ReactNode }> = ({ frames, children }) => {
  const f = useCurrentFrame();
  const o = frames > 0 ? interpolate(f, [0, frames], [0, 1], { extrapolateRight: "clamp" }) : 1;
  return <AbsoluteFill style={{ opacity: o }}>{children}</AbsoluteFill>;
};

export const Recreate: React.FC<RecreatePlan> = (plan) => {
  const { height } = useVideoConfig();
  const cs = plan.captionStyle;
  const musicFrom = Math.max(0, Math.round(-(plan.audio?.musicStart ?? 0) * plan.fps));
  // ducking: a musica abaixa onde um plano toca a fala real do clipe (rampa de 5 frames)
  const talk = [
    ...plan.shots.filter((s) => (s.volume ?? 0) > 0).map((s) => [s.from, s.to] as const),
    ...(plan.voiceClips ?? []).filter((v) => v.from !== undefined).map((v) => [v.from!, v.to!] as const),
  ];
  const duckLevel = plan.audio?.duck ?? 0.35;
  const duckAt = (f: number) => {
    let g = 1;
    for (const [a, b] of talk) {
      const k = interpolate(f, [a - 5, a, b, b + 5], [0, 1, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
      g = Math.min(g, 1 - k * (1 - duckLevel));
    }
    return g;
  };
  const fx = Object.fromEntries(Object.entries(plan.frameFx ?? {}).map(([k, v]) => [Number(k), v])) as Record<
    number,
    FrameFxKind
  >;
  return (
    <AbsoluteFill style={{ backgroundColor: "#000" }}>
      <FrameFx fx={fx}>
        <MotionFx events={plan.motion ?? []} seed={plan.id} glow={plan.look?.deepGlow}>
          {plan.shots.map((s, i) => {
            const d = s.transitionIn?.kind === "dissolve" ? s.transitionIn.frames : 0;
            const from = Math.max(0, s.from - d);
            const len = s.to - from;
            const g = s.grade;
            return (
              <Sequence key={i} from={from} durationInFrames={len}>
                <DissolveIn frames={s.from - from}>
                  <AbsoluteFill style={{ filter: SHOT_FX[s.fx ?? "none"] }}>
                  <Graded
                    id={`rg-${i}`}
                    spec={
                      g
                        ? levels({ black: g.black, white: g.white, gamma: g.gamma, saturate: g.saturate, sharpen: g.sharpen, tint: g.tint })
                        : levels({ black: 0, white: 255 })
                    }
                  >
                    {s.layers ? (
                      <Parallax bg={s.layers.bg} fg={s.layers.fg} mode={s.layers.mode} amount={s.layers.amount}
                        dir={s.layers.dir} durationInFrames={len} />
                    ) : (
                    <Shot
                      volume={s.volume}
                      src={s.src}
                      startAt={s.startAt}
                      rate={s.rate}
                      freeze={s.freeze}
                      durationInFrames={len}
                      camera={s.camera}
                      jitter={s.jitter ? { ...s.jitter, seed: `rs${i}` } : undefined}
                      overscan={s.overscan}
                      objectPosition={s.objectPosition}
                    />
                    )}
                  </Graded>
                  </AbsoluteFill>
                  {plan.look?.glow ? (
                    <AbsoluteFill style={{ mixBlendMode: "screen", opacity: plan.look.glow, filter: "blur(8px) brightness(1.2)" }}>
                      {s.layers ? (
                        <Parallax bg={s.layers.bg} fg={s.layers.fg} mode={s.layers.mode} amount={s.layers.amount}
                          dir={s.layers.dir} durationInFrames={len} />
                      ) : (
                        <Shot src={s.src} startAt={s.startAt} rate={s.rate} freeze={s.freeze} durationInFrames={len}
                          camera={s.camera} overscan={s.overscan} objectPosition={s.objectPosition}
                          // mesmo tremor (mesma seed) da imagem de baixo: sem isso o brilho saia deslocado (fantasma)
                          jitter={s.jitter ? { ...s.jitter, seed: `rs${i}` } : undefined} />
                      )}
                    </AbsoluteFill>
                  ) : null}
                </DissolveIn>
              </Sequence>
            );
          })}
          {(plan.blooms ?? []).map((b, i) => (
            <Sequence key={`b${i}`} from={b.from} durationInFrames={b.to - b.from}>
              <LightBloom keys={b.keys} sources={b.sources} />
            </Sequence>
          ))}
        </MotionFx>
        <Vignette strength={plan.look?.vignette ?? 0.4} inner={plan.look?.vignetteInner ?? 45} />
        <FilmGrain opacity={plan.look?.grain ?? 0.25} size={plan.look?.grainSize ?? 0.85} />
        {plan.title && plan.title.steps.length ? (
          <TitleStack
            steps={plan.title.steps}
            end={plan.title.end}
            fontFamily={font(plan.title.font ?? "LeagueGothic")}
            capHeight={plan.title.capHeight ?? 0.6}
            scaleX={plan.title.scaleX ?? 0.45}
            scaleY={plan.title.scaleY ?? 1.55}
            y={plan.title.y ?? 0.5}
            blend={(plan.title.blend ?? "difference") as React.CSSProperties["mixBlendMode"]}
            glowPx={plan.title.glowPx ?? 10}
            glowOpacity={plan.title.glowOpacity ?? 0.8}
            fringePx={plan.title.fringePx ?? 0}
            color={plan.title.color ?? "#ffffff"}
          />
        ) : null}
        {plan.watermark?.text ? (
          <Sequence from={plan.watermark.from ?? 0}>
            <Watermark
              text={plan.watermark.text}
              fontFamily={font(plan.watermark.font ?? "Arimo")}
              sizePx={plan.watermark.sizePx ?? height * 0.0215}
              y={plan.watermark.y}
              color={plan.watermark.color}
              opacity={plan.watermark.opacity}
            />
          </Sequence>
        ) : null}
      </FrameFx>
      {(plan.captions ?? []).map((c, i, all) => (
        <CaptionTrack
          key={c.seed ?? i}
          steps={c.steps as CaptionStep[]}
          end={Math.min(c.end, all[i + 1]?.steps[0]?.f ?? Infinity)}
          twitch={cs.twitch ? { seed: c.seed ?? `c${i}`, blank: cs.twitch.blank, back: cs.twitch.back } : undefined}
          stretch={cs.stretch ?? undefined}
          opacity={cs.opacity}
          pop={cs.pop ?? undefined}
          style={{
            fontFamily: font(cs.font),
            weight: cs.weight,
            italic: cs.italic,
            sizePx: cs.sizePx,
            x: cs.x,
            y: cs.y,
            color: cs.color,
            strokePx: cs.strokePx,
            strokeColor: cs.strokeColor,
            shadow: cs.shadow,
            letterSpacing: cs.letterSpacing,
            upper: cs.upper ?? true,
            maxWidth: cs.maxWidth,
            lineHeight: cs.lineHeight,
          }}
        />
      ))}
      {plan.endCard ? (
        <Sequence from={plan.endCard.from}>
          <AbsoluteFill style={{ backgroundColor: plan.endCard.background ?? "#000", justifyContent: "center", alignItems: "center" }}>
            <div style={{ fontFamily: font(plan.endCard.font ?? "RobotoCondensed"), fontWeight: 700,
              fontSize: plan.endCard.sizePx ?? height * 0.04, color: plan.endCard.color ?? "#fff" }}>
              {plan.endCard.text}
            </div>
          </AbsoluteFill>
        </Sequence>
      ) : null}
      {plan.audio?.music ? (
        // musicStart < 0: a trilha entra atrasada (o drop dela cai depois do inicio do video)
        <Sequence from={musicFrom}>
          <Audio
            src={staticFile(plan.audio.music)}
            volume={(f) => (plan.audio?.musicVolume ?? 1) * duckAt(f + musicFrom)}
            trimBefore={Math.max(0, Math.round((plan.audio.musicStart ?? 0) * plan.fps))}
          />
        </Sequence>
      ) : null}
      {(plan.voiceClips ?? []).map((v, i) =>
        v.from !== undefined && v.to !== undefined ? (
          <Sequence key={`vc${i}`} from={v.from} durationInFrames={v.to - v.from}>
            <Audio src={staticFile(v.src)} trimBefore={Math.round((v.startAt ?? 0) * plan.fps)} volume={v.volume ?? 1} />
          </Sequence>
        ) : null,
      )}
      {plan.audio?.voice ? <Audio src={staticFile(plan.audio.voice)} volume={plan.audio.voiceVolume ?? 1} /> : null}
    </AbsoluteFill>
  );
};
