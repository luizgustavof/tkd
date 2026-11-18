"""
Passe automatico de movimento: deixa o edit mais envolvente sem o agente ter que escrever cada evento.

uso: python scripts/automotion.py PLAN.json PUBLIC_DIR [--preset high|medium|low|none]
Le shots/frameFx/audio do plano, mede os onsets REAIS da trilha (librosa) na janela usada e grava:
  plan["motion"]          eventos punch/pulse/shake/rgb/flash/blur (frames absolutos)
  shots[i]["camera"]      push-in lento nos planos longos sem camera
  captionStyle["pop"]     entrada com pop da legenda (high/medium)
Eventos que o agente ja escreveu em plan["motion"] sao mantidos; preset vem de plan["motionPreset"] se existir.

Presets (por energia do canal):
  high   (hype)           punch em todo corte rapido, shake+rgb nos cortes em onset forte, flash+punch no gatilho,
                           pulse nas batidas dentro de planos longos, push-in 1.00->1.05
  medium                   punch leve nos cortes em onset, pulse raro, push-in 1.00->1.04
  low    (calmo)           so push-in 1.00->1.04 (sem impacto)
"""
import argparse, json, os
import numpy as np

PRESETS = {
    "high": dict(punch=1.10, punch_dur=5, shake=9, rgb=7, pulse=1.025, push=1.05, pop=(1.12, 3), trigger=True),
    "medium": dict(punch=1.06, punch_dur=5, shake=0, rgb=4, pulse=1.015, push=1.04, pop=(1.08, 3), trigger=True),
    "low": dict(punch=0, punch_dur=0, shake=0, rgb=0, pulse=0, push=1.04, pop=None, trigger=False),
    "none": dict(punch=0, punch_dur=0, shake=0, rgb=0, pulse=0, push=0, pop=None, trigger=False),
}


def strong_onsets(path, start, dur, fps, offset_frames):
    """frames (na composicao) dos onsets fortes da trilha no trecho [start, start+dur]."""
    try:
        import librosa
    except ImportError:
        return []
    y, sr = librosa.load(path, sr=22050, offset=max(0.0, start), duration=dur + 0.5)
    env = librosa.onset.onset_strength(y=y, sr=sr)
    peaks = librosa.onset.onset_detect(onset_envelope=env, sr=sr, units="frames")
    if len(peaks) == 0:
        return []
    thr = np.quantile(env[peaks], 0.6)
    ts = librosa.frames_to_time([p for p in peaks if env[p] >= thr], sr=sr)
    return [int(round(t * fps)) + offset_frames for t in ts]


def near(f, frames, tol=2):
    return any(abs(f - g) <= tol for g in frames)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("plan")
    ap.add_argument("public_dir")
    ap.add_argument("--preset")
    a = ap.parse_args()
    plan = json.load(open(a.plan, encoding="utf-8"))
    preset = a.preset or plan.get("motionPreset") or "medium"
    P = PRESETS[preset]
    fps = plan.get("fps", 30)
    total = plan.get("durationInFrames") or max(s["to"] for s in plan["shots"])
    shots = plan["shots"]
    events = list(plan.get("motion") or [])
    agent_frames = {e["f"] for e in events}

    au = plan.get("audio") or {}
    onsets = []
    if au.get("music"):
        ms = au.get("musicStart", 0) or 0
        onsets = strong_onsets(os.path.join(a.public_dir, au["music"]), max(0.0, ms), total / fps, fps,
                               max(0, round(-ms * fps)))
        onsets = [f for f in onsets if 0 < f < total]

    fx = {int(k): v for k, v in (plan.get("frameFx") or {}).items()}
    trigger = min([f for f, v in fx.items() if v in ("invert", "white")], default=None)

    def add(f, kind, amount, dur):
        if f in agent_frames or amount == 0 or dur == 0:
            return
        events.append({"f": int(f), "kind": kind, "amount": round(float(amount), 3), "dur": int(dur)})

    for i, s in enumerate(shots):
        n = s["to"] - s["from"]
        # push-in lento em plano longo parado sem camera definida
        if P["push"] and n >= fps and not s.get("camera") and not s.get("jitter"):
            s["camera"] = {"from": {"s": 1.0}, "to": {"s": P["push"]}, "easing": "linear"}
            s["overscan"] = max(s.get("overscan", 1.0), 1.0)
        if i == 0:
            continue
        f = s["from"]
        on_beat = near(f, onsets)
        fast = n < 0.6 * fps
        if P["punch"] and (fast or on_beat):
            add(f, "punch", P["punch"] if on_beat else 1 + (P["punch"] - 1) * 0.6, P["punch_dur"])
        if on_beat and P["shake"]:
            add(f, "shake", P["shake"], 6)
        if on_beat and P["rgb"]:
            add(f, "rgb", P["rgb"], 4)

    # batidas dentro de planos longos: pulso sutil (no maximo 1 a cada 12 frames, longe de cortes)
    if P["pulse"]:
        cuts = [s["from"] for s in shots]
        last = -99
        for f in onsets:
            if near(f, cuts, 4) or f - last < 12:
                continue
            add(f, "pulse", P["pulse"], 8)
            last = f

    # efeitos de 1 frame ganham tranco; o gatilho do drop ganha clarao + punch forte
    for f, v in fx.items():
        if P["rgb"]:
            add(f, "rgb", P["rgb"] * 1.4, 3)
        if P["shake"]:
            add(f, "shake", P["shake"] * 1.2, 5)
    if trigger is not None and P["trigger"]:
        add(trigger + 1, "flash", 0.75, 7)
        add(trigger + 1, "punch", 1.16, 9)
        add(trigger + 1, "blur", 6, 4)

    if P["pop"] and plan.get("captionStyle") is not None and "pop" not in plan["captionStyle"]:
        plan["captionStyle"]["pop"] = {"scale": P["pop"][0], "frames": P["pop"][1]}

    events.sort(key=lambda e: (e["f"], e["kind"]))
    plan["motion"] = events
    plan["motionPreset"] = preset
    json.dump(plan, open(a.plan, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    kinds = {}
    for e in events:
        kinds[e["kind"]] = kinds.get(e["kind"], 0) + 1
    print(f"automotion [{preset}]: {len(onsets)} onsets fortes, eventos {kinds}")


if __name__ == "__main__":
    main()
