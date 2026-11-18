// Trailer do projeto (README): 1920x1080, 30 fps, 54 s. Cortes no compasso da trilha (120 BPM = 60 frames).
// As cenas do meio sao gravacoes do app de verdade (dados de demonstracao, canais ficticios), com camera que
// segue o cursor e parallax em camadas: fundo, janela 3D, chips e legenda andam em profundidades diferentes.
import React from "react";
import {
  AbsoluteFill,
  Audio,
  Easing,
  Img,
  interpolate,
  OffthreadVideo,
  Sequence,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { loadFont } from "@remotion/google-fonts/Geist";
import { loadFont as loadMono } from "@remotion/google-fonts/GeistMono";
import cNew from "../../public/trailer/rec-newchannel.cursor.json";
import cNotes from "../../public/trailer/rec-notes.cursor.json";
import cLive from "../../public/trailer/rec-live.cursor.json";
import cAgents from "../../public/trailer/rec-agents.cursor.json";
import cLang from "../../public/trailer/rec-language.cursor.json";

const { fontFamily: SANS } = loadFont("normal", { weights: ["400", "500", "600", "700"], subsets: ["latin"] });
const { fontFamily: MONO } = loadMono("normal", { weights: ["400", "500"], subsets: ["latin"] });

const BAR = 60;
const FPS = 30;
const BG = "#0e0f11";
const ACCENT = "#8b8cff";
const INK = "#ededf0";
const MUTED = "#8a8d96";

export const TRAILER_FRAMES = 27 * BAR;

const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

// ------------------------------------------------------------------ pecas

/** Marca do app: tres barras (branca, roxa, cinza) que entram no tempo. */
const Logo: React.FC<{ size: number; delay?: number }> = ({ size, delay = 0 }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const bars = [
    { w: 0.8, c: "#f0f0f2", y: 0.12 },
    { w: 0.55, c: ACCENT, y: 0.42 },
    { w: 0.7, c: "#666a73", y: 0.72 },
  ];
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" style={{ overflow: "visible" }}>
      {bars.map((b, i) => {
        const s = spring({ frame: frame - delay - i * 15, fps, config: { damping: 16, mass: 0.7 } });
        return <rect key={i} x={4} y={b.y * 100} height={16} width={Math.max(0, b.w * 92 * s)} fill={b.c} rx={1.5} />;
      })}
    </svg>
  );
};

/** Fundo em tres camadas com velocidades diferentes (parallax): brilho, grade e particulas. */
const Backdrop: React.FC<{ px?: number; py?: number }> = ({ px = 0, py = 0 }) => {
  const frame = useCurrentFrame();
  const drift = frame / 2;
  return (
    <AbsoluteFill style={{ background: BG, overflow: "hidden" }}>
      <AbsoluteFill style={{ transform: `translate(${px * 0.15}px, ${py * 0.15}px)` }}>
        <div style={{ position: "absolute", left: 1100 + Math.sin(frame / 70) * 120, top: 120 + Math.cos(frame / 90) * 80, width: 900, height: 900, borderRadius: 999, background: "radial-gradient(circle, rgba(139,140,255,0.20), transparent 62%)" }} />
        <div style={{ position: "absolute", left: -200 + Math.cos(frame / 80) * 100, top: 500 + Math.sin(frame / 60) * 60, width: 800, height: 800, borderRadius: 999, background: "radial-gradient(circle, rgba(62,207,142,0.08), transparent 62%)" }} />
      </AbsoluteFill>
      <AbsoluteFill
        style={{
          transform: `translate(${px * 0.3}px, ${py * 0.3}px)`,
          backgroundImage:
            "linear-gradient(rgba(255,255,255,0.035) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.035) 1px, transparent 1px)",
          backgroundSize: "80px 80px",
          backgroundPosition: `${-drift}px ${-drift / 2}px`,
          maskImage: "radial-gradient(ellipse at center, black 15%, transparent 72%)",
        }}
      />
      <AbsoluteFill style={{ transform: `translate(${px * 0.6}px, ${py * 0.6}px)` }}>
        {Array.from({ length: 26 }, (_, i) => {
          const x = (i * 377) % 1920;
          const y = ((i * 211 + frame * (0.4 + (i % 5) * 0.15)) % 1240) - 80;
          const s = 2 + (i % 4);
          return <span key={i} style={{ position: "absolute", left: x, top: 1080 - y, width: s, height: s, borderRadius: 9, background: i % 3 ? "rgba(255,255,255,0.18)" : "rgba(139,140,255,0.45)" }} />;
        })}
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

/** Texto que sobe palavra por palavra, por tras de uma mascara. */
const Rise: React.FC<{ text: string; size: number; delay?: number; weight?: number; color?: string; stagger?: number }> = ({
  text,
  size,
  delay = 0,
  weight = 600,
  color = INK,
  stagger = 3,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ display: "flex", flexWrap: "wrap", columnGap: size * 0.26, fontFamily: SANS, fontSize: size, fontWeight: weight, color, letterSpacing: -size * 0.03, lineHeight: 1.08 }}>
      {text.split(" ").map((w, i) => {
        const s = spring({ frame: frame - delay - i * stagger, fps, config: { damping: 18, mass: 0.6 } });
        return (
          <span key={i} style={{ overflow: "hidden", display: "inline-block", paddingBottom: size * 0.08 }}>
            <span style={{ display: "inline-block", transform: `translateY(${(1 - s) * 110}%)` }}>{w}</span>
          </span>
        );
      })}
    </div>
  );
};

const Kicker: React.FC<{ text: string; delay?: number; color?: string }> = ({ text, delay = 0, color = MUTED }) => {
  const frame = useCurrentFrame();
  const o = interpolate(frame - delay, [0, 10], [0, 1], clamp);
  return (
    <div style={{ fontFamily: MONO, fontSize: 20, letterSpacing: 4, color, opacity: o, transform: `translateX(${(1 - o) * -12}px)` }}>
      {text}
    </div>
  );
};

// ------------------------------------------------------------------ cursor e camera

type Mark = { t: number; x: number; y: number; k: string };
const ease = Easing.bezier(0.45, 0, 0.2, 1);

function cursorAt(marks: Mark[], t: number): { x: number; y: number } {
  if (!marks.length) return { x: 960, y: 600 };
  if (t <= marks[0]!.t) return marks[0]!;
  for (let i = 1; i < marks.length; i++) {
    const a = marks[i - 1]!;
    const b = marks[i]!;
    if (t <= b.t) {
      const p = b.t > a.t ? ease((t - a.t) / (b.t - a.t)) : 1;
      return { x: a.x + (b.x - a.x) * p, y: a.y + (b.y - a.y) * p };
    }
  }
  return marks.at(-1)!;
}

/** Ponto que a camera olha: media do cursor numa janela de 1,6 s (anda macio, chega antes do clique). */
function lookAt(marks: Mark[], t: number) {
  let x = 0;
  let y = 0;
  const n = 17;
  for (let i = 0; i < n; i++) {
    const c = cursorAt(marks, t - 0.5 + (i / (n - 1)) * 1.6);
    x += c.x;
    y += c.y;
  }
  return { x: x / n, y: y / n };
}

const Arrow: React.FC = () => (
  <svg width="30" height="30" viewBox="0 0 24 24" style={{ filter: "drop-shadow(0 2px 4px rgba(0,0,0,.6))" }}>
    <path d="M4 2.5 L4 19 L8.6 14.8 L11.6 21.5 L14.4 20.3 L11.5 13.8 L17.8 13.6 Z" fill="#fff" stroke="#111" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);

// ------------------------------------------------------------------ cena gravada

type Rec = {
  id: string;
  marks: Mark[];
  /** segundo da gravacao em que a cena comeca */
  from: number;
  rate: number;
  zoom: number;
  kicker: string;
  title: string;
  chips: { text: string; x: number; y: number; depth: number }[];
};

const RECS: Rec[] = [
  { id: "newchannel", marks: cNew as Mark[], from: 1.6, rate: 1.9, zoom: 1.55, kicker: "01 · NEW CHANNEL", title: "A new channel in a minute.",
    chips: [{ text: "4 steps", x: 1540, y: 150, depth: 1.2 }, { text: "one Edge window per channel", x: 1290, y: 905, depth: 1.6 }] },
  { id: "notes", marks: cNotes as Mark[], from: 0.9, rate: 1.9, zoom: 1.6, kicker: "02 · NOTES", title: "Tell the agent what you want.",
    chips: [{ text: "read before every video", x: 1330, y: 160, depth: 1.4 }] },
  { id: "live", marks: cLive as Mark[], from: 0.8, rate: 1.85, zoom: 1.4, kicker: "03 · LIVE", title: "Watch every agent work.",
    chips: [{ text: "● REC  chat + audio peaks", x: 1300, y: 150, depth: 1.5 }] },
  { id: "agents", marks: cAgents as Mark[], from: 0.7, rate: 2, zoom: 1.45, kicker: "04 · AGENTS", title: "Pick the model per task.",
    chips: [{ text: "Opus · Sonnet · Haiku", x: 1400, y: 150, depth: 1.3 }] },
  { id: "language", marks: cLang as Mark[], from: 0.6, rate: 1.95, zoom: 1.25, kicker: "05 · LANGUAGE", title: "Português · English · العربية",
    chips: [{ text: "right-to-left ready", x: 1420, y: 150, depth: 1.5 }] },
];
const REC_BARS = [5, 2, 2, 2, 3];

/** Janela do app (1920x1200 gravado) em 3D, com camera que segue o cursor. */
const RecShot: React.FC<{ r: Rec; len: number }> = ({ r, len }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const t = r.from + (frame / FPS) * r.rate;
  // zoom entra depois da janela pousar e sai no fim
  const zin = interpolate(frame, [10, 34], [1, r.zoom], { ...clamp, easing: ease });
  const zout = interpolate(frame, [len - 16, len], [0, 1], { ...clamp, easing: ease });
  const z = zin - (zin - 1) * zout;
  const look = lookAt(r.marks, t);
  const W = 1500;
  const H = 937.5;
  const s = (W / 1920) * z;
  const ox = Math.min(0, Math.max(W - 1920 * s, W / 2 - look.x * s));
  const oy = Math.min(0, Math.max(H - 1200 * s, H / 2 - look.y * s));
  // camera: o quanto a janela "andou" vira o deslocamento do parallax das outras camadas
  const camX = ox + (1920 * s - W) / 2;
  const camY = oy + (1200 * s - H) / 2;
  const land = spring({ frame, fps, config: { damping: 15, mass: 0.9 } });
  const rotY = interpolate(land, [0, 1], [-18, 0]) + Math.sin(frame / 38) * 3.2 + camX * 0.004;
  const rotX = interpolate(land, [0, 1], [22, 0]) + Math.cos(frame / 45) * 2.2 - camY * 0.004;
  const ty = interpolate(land, [0, 1], [220, 0]);
  const out = interpolate(frame, [len - 8, len], [1, 0], clamp);
  const cur = cursorAt(r.marks, t);
  const clicks = r.marks.filter((m) => m.k === "click" && t >= m.t && t - m.t < 0.45);

  return (
    <AbsoluteFill style={{ opacity: out }}>
      <Backdrop px={camX * 0.12} py={camY * 0.12} />
      <AbsoluteFill style={{ perspective: 2400, alignItems: "center", justifyContent: "center" }}>
        <div
          style={{
            width: W,
            height: H,
            marginTop: -40,
            transform: `translateY(${ty}px) rotateX(${rotX}deg) rotateY(${rotY}deg) scale(${interpolate(land, [0, 1], [0.86, 1])})`,
            transformStyle: "preserve-3d",
            borderRadius: 18,
            overflow: "hidden",
            background: BG,
            boxShadow: "0 0 0 1px rgba(255,255,255,0.09), 0 40px 120px rgba(0,0,0,0.7), 0 0 160px rgba(139,140,255,0.16)",
          }}
        >
          <div style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1200, transformOrigin: "0 0", transform: `translate(${ox}px, ${oy}px) scale(${s})` }}>
            <OffthreadVideo src={staticFile(`trailer/rec-${r.id}.mp4`)} trimBefore={Math.round(r.from * FPS)} playbackRate={r.rate} muted style={{ width: 1920, height: 1200 }} />
            {clicks.map((c) => {
              const p = (t - c.t) / 0.45;
              return (
                <span
                  key={c.t}
                  style={{ position: "absolute", left: c.x - 40 * p, top: c.y - 40 * p, width: 80 * p, height: 80 * p, borderRadius: 999, border: `3px solid rgba(139,140,255,${1 - p})`, background: `rgba(139,140,255,${0.25 * (1 - p)})` }}
                />
              );
            })}
            <div style={{ position: "absolute", left: cur.x - 4, top: cur.y - 3, transform: `scale(${1 / Math.sqrt(z)})`, transformOrigin: "4px 3px" }}>
              <Arrow />
            </div>
          </div>
          {/* reflexo de vidro que corre com a inclinacao */}
          <div style={{ position: "absolute", inset: 0, pointerEvents: "none", background: `linear-gradient(${115 + rotY * 3}deg, transparent 35%, rgba(255,255,255,0.05) 50%, transparent 65%)` }} />
        </div>
      </AbsoluteFill>
      {/* chips na frente: andam mais que a janela (mais perto da camera) */}
      {r.chips.map((c, i) => {
        const sp = spring({ frame: frame - 18 - i * 8, fps, config: { damping: 14 } });
        return (
          <div
            key={c.text}
            style={{
              position: "absolute",
              left: c.x - camX * 0.18 * c.depth,
              top: c.y - camY * 0.18 * c.depth + Math.sin(frame / 25 + i) * 6 + (1 - sp) * 40,
              opacity: sp,
              padding: "12px 18px",
              borderRadius: 12,
              background: "rgba(24,25,30,0.72)",
              backdropFilter: "blur(14px)",
              boxShadow: "0 0 0 1px rgba(139,140,255,0.35), 0 18px 40px rgba(0,0,0,0.55)",
              fontFamily: MONO,
              fontSize: 20,
              color: "#c9caff",
              whiteSpace: "nowrap",
            }}
          >
            {c.text}
          </div>
        );
      })}
      {/* legenda: a camada mais proxima */}
      <AbsoluteFill style={{ background: "linear-gradient(10deg, rgba(8,8,10,0.92) 12%, rgba(8,8,10,0.45) 32%, transparent 52%)" }} />
      <div style={{ position: "absolute", left: 110 - camX * 0.05, bottom: 92 - camY * 0.05, width: 1200, display: "flex", flexDirection: "column", gap: 18 }}>
        <Kicker text={r.kicker} delay={8} color={ACCENT} />
        <Rise text={r.title} size={74} delay={10} />
      </div>
    </AbsoluteFill>
  );
};

// ------------------------------------------------------------------ intro, cartoes, fim

const Intro: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const move = spring({ frame: frame - 120, fps, config: { damping: 20 } });
  const size = interpolate(move, [0, 1], [300, 190]);
  const x = interpolate(move, [0, 1], [0, -330]);
  const glow = interpolate(frame, [0, 40, 120], [0, 1, 0.6], clamp);
  const flash = interpolate(frame, [232, 240], [0, 1], clamp);
  // parallax lento: a camera chega perto durante a intro
  const push = interpolate(frame, [0, 240], [0, 1]);
  return (
    <AbsoluteFill>
      <Backdrop px={-push * 120} py={-push * 40} />
      <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", transform: `scale(${1 + push * 0.06})` }}>
        <div style={{ position: "absolute", width: 700, height: 700, borderRadius: 999, background: `radial-gradient(circle, rgba(139,140,255,${0.28 * glow}), transparent 65%)` }} />
        <div style={{ transform: `translateX(${x}px)`, filter: `drop-shadow(0 0 ${30 * glow}px rgba(139,140,255,0.5))` }}>
          <Logo size={size} delay={4} />
        </div>
        <Sequence from={128} layout="none">
          <div style={{ position: "absolute", left: 960 - 120, top: 540 - 120 }}>
            <Rise text="tkd" size={150} weight={700} />
          </div>
        </Sequence>
        <Sequence from={150} layout="none">
          <div style={{ position: "absolute", left: 960 - 520, top: 540 + 130, width: 1100 }}>
            <Rise text="A factory of TikTok & Shorts channels." size={46} weight={500} color="#c9cad1" stagger={2} />
          </div>
        </Sequence>
        <Sequence from={180} layout="none">
          <div style={{ position: "absolute", left: 960 - 520, top: 540 + 205 }}>
            <Kicker text="RUST · TAURI · CLAUDE · REMOTION" />
          </div>
        </Sequence>
      </AbsoluteFill>
      <AbsoluteFill style={{ background: "white", opacity: flash * 0.85 }} />
    </AbsoluteFill>
  );
};

type Card = { n: string; title: string; sub: string; img: string };
const CARDS: Card[] = [
  { n: "01", title: "TikTok + YouTube.", sub: "ONE EDGE PROFILE PER CHANNEL", img: "channel-clips" },
  { n: "02", title: "Twitch · Kick · YouTube.", sub: "CHAT + AUDIO + MOTION PEAKS", img: "live" },
  { n: "03", title: "Bring your own model.", sub: "PLUG A LOCAL EVALUATOR", img: "setup" },
  { n: "04", title: "Measured QA.", sub: "FFMPEG · WHISPER · LOUDNESS · SCENES", img: "channel-edit" },
  { n: "05", title: "Runs on your PC.", sub: "CPU ONLY · YOUR ACCOUNTS STAY HOME", img: "channels" },
  { n: "06", title: "Open source.", sub: "MIT LICENSE", img: "agents" },
];

/** Cartao rapido em tres planos: tela borrada (longe), numero gigante (meio) e titulo (perto). */
const QuickCard: React.FC<{ c: Card }> = ({ c }) => {
  const frame = useCurrentFrame();
  const p = frame / BAR;
  const fadeOut = interpolate(frame, [BAR - 4, BAR], [1, 0], clamp);
  return (
    <AbsoluteFill style={{ background: BG, opacity: fadeOut, overflow: "hidden" }}>
      <Img
        src={staticFile(`trailer/${c.img}.png`)}
        style={{ position: "absolute", width: 1920, height: 1200, top: -60, transform: `translateX(${-p * 40}px) scale(1.2)`, filter: "blur(14px) brightness(0.32) saturate(0.8)" }}
      />
      <div style={{ position: "absolute", right: 60 + p * 140, top: 60, fontFamily: SANS, fontWeight: 700, fontSize: 620, lineHeight: 1, color: "rgba(139,140,255,0.08)", letterSpacing: -30 }}>
        {c.n}
      </div>
      <div style={{ position: "absolute", left: 110 - p * 200 * 0.35, top: 0, bottom: 0, display: "flex", flexDirection: "column", justifyContent: "center", gap: 22 }}>
        <Kicker text={c.n} color={ACCENT} />
        <Rise text={c.title} size={110} delay={2} weight={700} stagger={2} />
        <Kicker text={c.sub} delay={10} />
      </div>
    </AbsoluteFill>
  );
};

const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const fade = interpolate(frame, [3 * BAR - 30, 3 * BAR], [1, 0], clamp);
  const flash = interpolate(frame, [0, 10], [0.9, 0], clamp);
  const pull = interpolate(frame, [0, 3 * BAR], [0, 1]);
  return (
    <AbsoluteFill style={{ opacity: fade }}>
      <Backdrop px={pull * 160} py={pull * 50} />
      <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", transform: `scale(${1.04 - pull * 0.04})` }}>
        <div style={{ display: "flex", alignItems: "center", gap: 40, marginTop: -80 }}>
          <Logo size={170} delay={2} />
          <Rise text="tkd" size={170} weight={700} delay={10} />
        </div>
        <div style={{ position: "absolute", top: 540 + 90, display: "flex", flexDirection: "column", alignItems: "center", gap: 22 }}>
          <Sequence from={30} layout="none">
            <Rise text="Open source now." size={48} weight={500} color="#c9cad1" />
          </Sequence>
          <Sequence from={45} layout="none">
            <Kicker text="GITHUB.COM/LUIZGUSTAVOF/TKD" color={ACCENT} />
          </Sequence>
        </div>
      </AbsoluteFill>
      <AbsoluteFill style={{ background: "white", opacity: flash }} />
    </AbsoluteFill>
  );
};

export const Trailer: React.FC = () => {
  const recStart = 4 * BAR;
  const starts = REC_BARS.reduce<number[]>((a, b, i) => [...a, i ? a[i - 1]! + REC_BARS[i - 1]! * BAR : recStart], []);
  const cardsStart = recStart + REC_BARS.reduce((a, b) => a + b, 0) * BAR; // compasso 18
  const outroStart = cardsStart + CARDS.length * BAR; // compasso 24
  return (
    <AbsoluteFill style={{ background: BG }}>
      <Sequence durationInFrames={recStart}>
        <Intro />
      </Sequence>
      {RECS.map((r, i) => (
        <Sequence key={r.id} from={starts[i]} durationInFrames={REC_BARS[i]! * BAR}>
          <RecShot r={r} len={REC_BARS[i]! * BAR} />
        </Sequence>
      ))}
      {CARDS.map((c, i) => (
        <Sequence key={c.n} from={cardsStart + i * BAR} durationInFrames={BAR}>
          <QuickCard c={c} />
        </Sequence>
      ))}
      <Sequence from={outroStart}>
        <Outro />
      </Sequence>
      <Audio src={staticFile("trailer/music.mp3")} volume={(f) => interpolate(f, [TRAILER_FRAMES - 45, TRAILER_FRAMES], [1, 0], clamp)} />
    </AbsoluteFill>
  );
};
