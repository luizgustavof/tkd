import React from "react";
import { AbsoluteFill, Img, interpolate, staticFile, useCurrentFrame } from "remotion";
import { ease } from "./core";

export type ParallaxProps = {
  /** fundo preenchido (sem o personagem), relativo a public/ */
  bg: string;
  /** personagem recortado (PNG com alfa), relativo a public/ */
  fg: string;
  durationInFrames: number;
  /** pan: camera anda de lado, frente anda mais que o fundo; dolly: personagem parado, fundo cresce (Vertigo) */
  mode?: "pan" | "dolly";
  /** forca do movimento (1 = padrao) */
  amount?: number;
  /** direcao do pan: 1 = da esquerda p/ direita, -1 = ao contrario */
  dir?: number;
};

/** Parallax 2.5D com duas camadas (scripts/parallax_layers.py gera bg/fg). */
export const Parallax: React.FC<ParallaxProps> = ({ bg, fg, durationInFrames, mode = "pan", amount = 1, dir = 1 }) => {
  const frame = useCurrentFrame();
  const t = interpolate(frame, [0, Math.max(1, durationInFrames - 1)], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: ease("inOut"),
  });
  const img: React.CSSProperties = { width: "100%", height: "100%", objectFit: "cover" };
  let bgT: string;
  let fgT: string;
  if (mode === "dolly") {
    // personagem do mesmo tamanho, fundo vai de 1.08 a ~1.5
    bgT = `scale(${(1.08 + 0.42 * amount * t).toFixed(4)})`;
    fgT = `scale(1.12)`;
  } else {
    // a sobra do zoom de cada camada esconde a borda: frente anda ate 55 px com zoom 1.12+, fundo 1/3 disso
    const x = (t * 2 - 1) * 55 * amount * dir;
    bgT = `translate(${(x * 0.33).toFixed(2)}px, 0px) scale(${(1.08 + 0.02 * t).toFixed(4)})`;
    fgT = `translate(${x.toFixed(2)}px, ${(-8 * amount * t).toFixed(2)}px) scale(${(1.12 + 0.08 * amount * t).toFixed(4)})`;
  }
  return (
    <AbsoluteFill style={{ overflow: "hidden", backgroundColor: "#000" }}>
      <AbsoluteFill style={{ transform: bgT, filter: "blur(1.5px)" }}>
        <Img src={staticFile(bg)} style={img} />
      </AbsoluteFill>
      <AbsoluteFill style={{ transform: fgT }}>
        <Img src={staticFile(fg)} style={img} />
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
