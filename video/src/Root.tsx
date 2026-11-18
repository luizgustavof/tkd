import React from "react";
import { Composition } from "remotion";
import { ChannelVideo } from "./Video";
import { defaultSpec, specSchema, type Spec } from "./schema";
import { Recreate } from "./recreate/Recreate";
import { emptyPlan } from "./recreate/types";
import { Trailer, TRAILER_FRAMES } from "./trailer/Trailer";

/** Um id de composicao por canal; o componente e o mesmo, o estilo vem da spec.
 *  `canal` = generico, padrao dos canais criados pelo app (id do Remotion nao aceita `_`). */
const CHANNELS = ["canal"] as const;

/** 1080 de largura em todos; a altura vem do formato que o canal usa. */
const DIMENSOES: Record<string, { width: number; height: number }> = {
  "9:16": { width: 1080, height: 1920 },
  "1:1": { width: 1080, height: 1080 },
  "4:5": { width: 1080, height: 1350 },
  "4:3": { width: 1080, height: 810 },
};

export const RemotionRoot: React.FC = () => (
  <>
    {CHANNELS.map((slug) => (
      <Composition
        key={slug}
        id={slug}
        component={ChannelVideo}
        schema={specSchema}
        defaultProps={{ ...defaultSpec, channel: slug } as Spec}
        width={1080}
        height={1920}
        fps={30}
        durationInFrames={360}
        calculateMetadata={({ props }) => ({
          durationInFrames: Math.round(props.durationS * props.fps),
          fps: props.fps,
          ...(DIMENSOES[props.aspect] ?? DIMENSOES["9:16"]),
        })}
      />
    ))}
    {/* trailer do projeto (README) */}
    <Composition id="trailer" component={Trailer} width={1920} height={1080} fps={30} durationInFrames={TRAILER_FRAMES} />
    {/* recriacao generica: props = plano gerado por `tkd recreate` */}
    <Composition
      id="recreate"
      component={Recreate}
      defaultProps={emptyPlan}
      width={1080}
      height={1080}
      fps={30}
      durationInFrames={60}
      calculateMetadata={({ props }) => ({
        durationInFrames: props.durationInFrames,
        fps: props.fps,
        width: props.width,
        height: props.height,
      })}
    />
  </>
);
