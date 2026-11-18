import React from "react";
import { AbsoluteFill, Freeze, Img, OffthreadVideo, staticFile, useCurrentFrame, useVideoConfig } from "remotion";
import { coverScale, ease, type EasingName, smoothNoise } from "./core";
import { interpolate } from "remotion";

export type CameraKey = { s?: number; x?: number; y?: number; rot?: number };

export type ShotProps = {
  /** caminho relativo a public/ (video ou imagem) */
  src: string;
  /** segundo de entrada no arquivo de origem */
  startAt?: number;
  /** velocidade de reproducao (0.25 = camera lenta) */
  rate?: number;
  /** congela no primeiro frame (plano "parado" de monologo) */
  freeze?: boolean;
  /** duracao do plano em frames: usada pelo movimento de camera */
  durationInFrames: number;
  /** movimento de camera: interpola from -> to ao longo do plano */
  camera?: { from?: CameraKey; to?: CameraKey; easing?: EasingName; delay?: number; speed?: number };
  /** respiracao/handheld: amplitude em px e frequencia em Hz */
  jitter?: { x?: number; y?: number; freq?: number; seed?: string };
  /** escala base para esconder bordas quando ha jitter/pan */
  overscan?: number;
  /** filtro CSS extra so deste plano */
  filter?: string;
  objectPosition?: string;
  /** volume do audio do proprio clipe (0 = mudo, padrao) — dialogo real (canal jane) */
  volume?: number;
};

const isVideo = (p: string) => /\.(mp4|webm|mov|mkv)$/i.test(p);

/** Plano de footage com camera virtual (zoom/pan/rotacao) e respiracao. */
export const Shot: React.FC<ShotProps> = ({
  src,
  startAt = 0,
  rate = 1,
  freeze,
  durationInFrames,
  camera,
  jitter,
  overscan = 1,
  filter,
  objectPosition = "50% 50%",
  volume = 0,
}) => {
  const frame = useCurrentFrame();
  const { fps, width, height } = useVideoConfig();

  const dur = Math.max(1, durationInFrames / (camera?.speed ?? 1));
  const t = interpolate(frame - (camera?.delay ?? 0), [0, dur], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: ease(camera?.easing ?? "linear"),
  });
  const a = camera?.from ?? {};
  const b = camera?.to ?? a;
  const lerp = (k: keyof CameraKey, def: number) => (a[k] ?? def) + ((b[k] ?? a[k] ?? def) - (a[k] ?? def)) * t;
  const s = lerp("s", 1) * overscan;
  let x = lerp("x", 0);
  let y = lerp("y", 0);
  const rot = lerp("rot", 0);
  if (jitter) {
    const sec = frame / fps;
    const seed = jitter.seed ?? src;
    x += smoothNoise(`${seed}x`, sec, jitter.freq ?? 0.6) * (jitter.x ?? 0);
    y += smoothNoise(`${seed}y`, sec, jitter.freq ?? 0.6) * (jitter.y ?? 0);
  }

  // pan/rotacao/jitter maiores que a sobra do zoom mostram o fundo preto: sobe o zoom ate cobrir o quadro
  const sCover = Math.max(s, coverScale(width, height, x, y, rot));

  const style: React.CSSProperties = { width: "100%", height: "100%", objectFit: "cover", objectPosition };
  const media = isVideo(src) ? (
    <OffthreadVideo
      src={staticFile(src)}
      muted={volume <= 0}
      volume={volume > 0 ? volume : undefined}
      playbackRate={rate}
      trimBefore={Math.round(startAt * fps)}
      style={style}
    />
  ) : (
    <Img src={staticFile(src)} style={style} />
  );

  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#000" }}>
      <AbsoluteFill
        style={{
          transform: `translate(${x}px, ${y}px) scale(${sCover}) rotate(${rot}deg)`,
          filter,
        }}
      >
        {freeze ? <Freeze frame={0}>{media}</Freeze> : media}
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
