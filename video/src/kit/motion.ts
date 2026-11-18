import { interpolate, spring, Easing } from "remotion";

/** Entrada com mola — usada pelos textos. */
export const pop = (frame: number, fps: number, delay = 0) =>
  spring({ frame: frame - delay, fps, config: { damping: 14, stiffness: 180, mass: 0.6 } });

/** Zoom lento continuo (Ken Burns). */
export const kenBurns = (
  frame: number,
  duration: number,
  mode: "none" | "slow-in" | "slow-out" | "punch"
) => {
  if (mode === "none") return 1;
  if (mode === "punch") return interpolate(frame, [0, 6, duration], [1.18, 1.0, 1.04], { extrapolateRight: "clamp" });
  const [from, to] = mode === "slow-in" ? [1.0, 1.12] : [1.12, 1.0];
  return interpolate(frame, [0, duration], [from, to], {
    extrapolateRight: "clamp",
    easing: Easing.inOut(Easing.ease),
  });
};

/** Tremor sincronizado ao corte. */
export const shake = (frame: number, amount = 8) => ({
  x: Math.sin(frame * 2.3) * amount,
  y: Math.cos(frame * 1.7) * amount * 0.6,
});
