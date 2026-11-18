import React from "react";
import {
  AbsoluteFill,
  Img,
  OffthreadVideo,
  interpolate,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import type { Spec } from "../schema";
import { fontFamily } from "./fonts";
import { kenBurns, pop, shake } from "./motion";
import { Highlighted, plainLength, sliceMarked, revealWords } from "./Text";
import { fitFontSize, fillFontSize } from "./fit";
import { TitleCard } from "./TitleCard";

type SceneProps = {
  scene: Spec["scenes"][number];
  spec: Spec;
};

const isVideo = (p: string) => /\.(mp4|webm|mov)$/i.test(p);

/** Uma cena: midia opcional ao fundo + texto animado por cima. */
export const Scene: React.FC<SceneProps> = ({ scene, spec }) => {
  const frame = useCurrentFrame();
  const { fps, height, width } = useVideoConfig();
  const dur = Math.round(scene.durationS * fps);

  const isTitle = scene.kind === "title";
  const scale = kenBurns(frame, dur, isTitle ? "punch" : spec.treatment.zoom);
  const sh = scene.effect === "shake" ? shake(frame) : { x: 0, y: 0 };

  // flash: frames de branco puro, sem lavar o resto da cena
  const flashFrames = Math.min(3, Math.max(2, Math.round(dur * 0.12)));
  const flash = scene.effect === "flash" && frame < flashFrames;
  // frame negativo: assinatura do drop
  const invert = scene.effect === "invert" && frame < Math.max(2, Math.round(dur * 0.35));

  const enter = pop(frame, fps, 1);
  const slide = scene.effect === "slide" ? interpolate(enter, [0, 1], [70, 0]) : 0;

  const progresso = interpolate(frame, [0, dur * 0.6], [0, 1], { extrapolateRight: "clamp" });
  const shown =
    scene.effect === "typewriter"
      ? sliceMarked(scene.text, Math.floor(progresso * plainLength(scene.text)))
      : scene.effect === "words"
        ? revealWords(scene.text, progresso)
        : scene.text;

  const upper = spec.typography.case === "upper";
  const texto = upper ? shown.toUpperCase() : shown;
  // mede sempre o texto COMPLETO: o corpo de letra nao pode pular durante o reveal
  const textoCheio = upper ? scene.text.toUpperCase() : scene.text;
  const familia = fontFamily(spec.typography.family);
  const baseVh = scene.sizeVh ?? spec.typography.sizeVh;
  const boxWidth = width * (isTitle ? 0.94 : 0.84);
  const boxHeight = height * (scene.sub ? 0.62 : 0.78);

  // cartela gigante ocupa a largura toda; legenda so nao pode estourar
  const preencher = scene.fill ?? (isTitle && baseVh >= 18);
  const size = preencher
    ? fillFontSize({
        text: textoCheio,
        fontFamily: familia,
        fontWeight: spec.typography.weight,
        boxWidth,
        boxHeight,
        lineHeight: 1.0,
      })
    : fitFontSize({
        text: textoCheio,
        fontFamily: familia,
        fontWeight: spec.typography.weight,
        desired: (height * baseVh) / 100,
        boxWidth,
        boxHeight,
        lineHeight: 1.05,
      });

  return (
    <AbsoluteFill style={{ backgroundColor: spec.palette.bg, overflow: "hidden" }}>
      <AbsoluteFill style={{ filter: invert ? "invert(1)" : undefined }}>
        {scene.media ? (
          <AbsoluteFill
            style={{
              transform: `scale(${scale})`,
              filter: spec.treatment.blurBg ? `blur(${spec.treatment.blurBg}px)` : undefined,
            }}
          >
            {isVideo(scene.media) ? (
              <OffthreadVideo
                src={staticFile(scene.media)}
                muted={!scene.volume}
                volume={scene.volume || undefined}
                trimBefore={scene.mediaStart ? Math.round(scene.mediaStart * fps) : undefined}
                style={{ width: "100%", height: "100%", objectFit: "cover" }}
              />
            ) : (
              <Img
                src={staticFile(scene.media)}
                style={{ width: "100%", height: "100%", objectFit: "cover" }}
              />
            )}
          </AbsoluteFill>
        ) : (
          <AbsoluteFill style={{ backgroundColor: spec.palette.bg }}>
            <AbsoluteFill
              style={{
                background: `radial-gradient(ellipse 70% 45% at 50% 42%, ${spec.palette.accent}1f 0%, transparent 70%)`,
                transform: `scale(${scale})`,
              }}
            />
          </AbsoluteFill>
        )}
      </AbsoluteFill>

      {flash ? <AbsoluteFill style={{ backgroundColor: "#fefefe" }} /> : null}

      {preencher ? (
        <AbsoluteFill
          style={{
            transform: `translate(${sh.x}px, ${sh.y}px) scale(${interpolate(
              enter,
              [0, 1],
              [0.93, 1]
            )})`,
            opacity: interpolate(enter, [0, 1], [0, 1]),
          }}
        >
          <TitleCard
            text={texto}
            fontFamily={familia}
            fontWeight={spec.typography.weight}
            fontSize={size}
            accent={spec.palette.accent}
          />
        </AbsoluteFill>
      ) : (
        <AbsoluteFill
          style={{
            justifyContent: "center",
            alignItems: "center",
            padding: "0 8%",
            transform: `translate(${sh.x}px, ${sh.y + slide}px)`,
            opacity: interpolate(enter, [0, 1], [0, 1]),
          }}
        >
          <div
            style={{
              fontFamily: familia,
              fontWeight: spec.typography.weight,
              fontStyle: spec.typography.style === "italic" ? "italic" : "normal",
              fontSize: size,
              lineHeight: 1.05,
              letterSpacing: upper ? "-0.01em" : "0",
              color: spec.palette.fg,
              textAlign: "center",
              whiteSpace: "pre-wrap",
              hyphens: "none",
              overflowWrap: "normal",
              wordBreak: "normal",
              textWrap: "balance" as never,
              textShadow: `0 4px 24px rgba(0,0,0,0.75)`,
              transform: `scale(${interpolate(enter, [0, 1], [0.96, 1])})`,
            }}
          >
            <Highlighted text={texto} accent={spec.palette.accent} upper={upper} />
          </div>
          {scene.sub ? (
            <div
              style={{
                marginTop: size * 0.35,
                fontFamily: familia,
                fontSize: size * 0.4,
                fontWeight: 600,
                color: spec.palette.fg,
                textAlign: "center",
                opacity: interpolate(frame, [dur * 0.08, dur * 0.2], [0, 0.9], {
                  extrapolateRight: "clamp",
                }),
              }}
            >
              {upper ? scene.sub.toUpperCase() : scene.sub}
            </div>
          ) : null}
        </AbsoluteFill>
      )}

      {/* faixa de glitch horizontal, so sobre cartela */}
      {preencher && spec.treatment.glitch && frame % 7 < 2 ? (
        <AbsoluteFill
          style={{
            top: `${35 + (frame % 3) * 8}%`,
            height: "6%",
            background: `${spec.palette.accent}55`,
            mixBlendMode: "screen",
          }}
        />
      ) : null}
    </AbsoluteFill>
  );
};
