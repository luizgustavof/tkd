import React from "react";
import { useVideoConfig } from "remotion";

type Props = {
  text: string;
  fontFamily: string;
  fontWeight: number;
  fontSize: number;
  accent: string;
  /** deslocamento da franja cromatica, em px */
  fringe?: number;
};

/**
 * Cartela de impacto com preenchimento cromado.
 * Feita em SVG de proposito: `background-clip: text` nao pinta no render
 * headless, o gradiente sumia e sobrava so a sombra.
 */
export const TitleCard: React.FC<Props> = ({
  text,
  fontFamily,
  fontWeight,
  fontSize,
  accent,
  fringe = 6,
}) => {
  const { width, height } = useVideoConfig();
  const linhas = text.split(/\s+/).filter(Boolean);
  const lineHeight = fontSize * 0.98;
  const totalH = linhas.length * lineHeight;
  const topo = height / 2 - totalH / 2 + lineHeight * 0.78;

  const comum = {
    textAnchor: "middle" as const,
    fontFamily,
    fontWeight,
    fontSize,
    letterSpacing: -fontSize * 0.03,
  };

  return (
    <svg width={width} height={height} style={{ position: "absolute", inset: 0 }}>
      <defs>
        <linearGradient id="cromo" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#ffffff" />
          <stop offset="34%" stopColor="#e4e8ef" />
          <stop offset="50%" stopColor="#8d929d" />
          <stop offset="58%" stopColor="#c9cdd6" />
          <stop offset="78%" stopColor="#ffffff" />
          <stop offset="100%" stopColor="#aeb4c0" />
        </linearGradient>
      </defs>
      {linhas.map((linha, i) => {
        const y = topo + i * lineHeight;
        return (
          <g key={i}>
            {/* franja cromatica: copias deslocadas por tras */}
            <text {...comum} x={width / 2 + fringe} y={y} fill={accent} opacity={0.9}>
              {linha}
            </text>
            <text {...comum} x={width / 2 - fringe * 0.7} y={y} fill="#5fd3ff" opacity={0.9}>
              {linha}
            </text>
            <text {...comum} x={width / 2} y={y} fill="url(#cromo)">
              {linha}
            </text>
          </g>
        );
      })}
    </svg>
  );
};
