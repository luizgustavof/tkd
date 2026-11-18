import React from "react";
import { AbsoluteFill, random, useCurrentFrame } from "remotion";

/** Grao de filme: ruido SVG re-semeado por frame. */
export const Grain: React.FC<{ opacity: number }> = ({ opacity }) => {
  const frame = useCurrentFrame();
  if (opacity <= 0) return null;
  const seed = random(`grain-${frame}`) * 100;
  return (
    <AbsoluteFill style={{ opacity: Math.min(0.5, opacity * 0.9), mixBlendMode: "screen", pointerEvents: "none" }}>
      <svg width="100%" height="100%">
        <filter id={`n${frame}`}>
          <feTurbulence type="fractalNoise" baseFrequency="0.75" numOctaves={4} seed={seed} />
        </filter>
        <rect width="100%" height="100%" filter={`url(#n${frame})`} />
      </svg>
    </AbsoluteFill>
  );
};

export const Vignette: React.FC<{ strength: number }> = ({ strength }) =>
  strength <= 0 ? null : (
    <AbsoluteFill
      style={{
        background: `radial-gradient(circle at 50% 45%, transparent 35%, rgba(0,0,0,${strength}) 100%)`,
        pointerEvents: "none",
      }}
    />
  );
