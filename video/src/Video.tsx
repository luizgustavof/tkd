import React from "react";
import { AbsoluteFill, Audio, Sequence, staticFile, useVideoConfig } from "remotion";
import type { Spec } from "./schema";
import { Grain, Vignette } from "./kit/Grain";
import { Scene } from "./kit/Scene";
import { fontFamily } from "./kit/fonts";

/** Composicao unica e parametrica: o estilo vem inteiro da spec do canal. */
export const ChannelVideo: React.FC<Spec> = (spec) => {
  const { fps, height } = useVideoConfig();
  const t = spec.treatment;
  // curva em S: preto esmagado e alto estourado, como nos edits de referencia
  const grade = `contrast(${1 + t.crush * 0.35}) brightness(${1 - t.crush * 0.08}) saturate(${1 + t.crush * 0.12})`;
  return (
    <AbsoluteFill style={{ backgroundColor: spec.palette.bg }}>
      <AbsoluteFill style={{ filter: grade }}>
      {spec.scenes.map((scene, i) => (
        <Sequence
          key={i}
          from={Math.round(scene.startS * fps)}
          durationInFrames={Math.max(1, Math.round(scene.durationS * fps))}
        >
          <Scene scene={scene} spec={spec} />
        </Sequence>
      ))}
      </AbsoluteFill>
      {t.bloom > 0 ? (
        <AbsoluteFill
          style={{
            backdropFilter: `blur(${12 * t.bloom}px) brightness(1.25)`,
            maskImage: "linear-gradient(transparent, transparent)",
            opacity: t.bloom * 0.5,
            mixBlendMode: "screen",
            pointerEvents: "none",
          }}
        />
      ) : null}
      <Vignette strength={t.vignette} />
      <Grain opacity={t.grain} />
      {spec.watermark?.text ? (
        <AbsoluteFill
          style={{
            justifyContent: "flex-end",
            alignItems: "center",
            paddingBottom: `${100 - 96.5}%`,
            pointerEvents: "none",
          }}
        >
          <div
            style={{
              fontFamily: fontFamily("Inter"),
              fontWeight: 700,
              fontSize: (height * spec.watermark.sizeVh) / 100,
              letterSpacing: "0.08em",
              color: spec.watermark.color,
              opacity: spec.watermark.opacity,
            }}
          >
            {spec.watermark.text}
          </div>
        </AbsoluteFill>
      ) : null}
      {spec.audio ? (
        <Audio
          src={staticFile(spec.audio)}
          volume={spec.audioVolume}
          trimBefore={spec.audioStart ? Math.round(spec.audioStart * fps) : undefined}
        />
      ) : null}
    </AbsoluteFill>
  );
};
