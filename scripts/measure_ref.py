"""
Medicao objetiva de uma referencia (complementa o rea-video, que perde cortes escuros).

uso: python scripts/measure_ref.py REF.mp4 OUT.json [--end SEG] [--rea DIR_DO_DECOMPOSE]
Saida:
  probe        {fps, width, height, duration}
  cuts         frames de corte seco (diff de pixel + histograma, limiar adaptativo)
  flashes      frames isolados de alto contraste (invertido/branco/P&B) — candidatos a FrameFx
  shots        [{from,to,dur_s, luma{p1..p99}, black_frac, clip_frac, sat, rgb, camera{scale_end, tx_end, ty_end, jitter_y}}]
  voice        falas do ASR do rea-video (texto, inicio, fim), se --rea
  music        {beat_s, drop_s (pico de onset mais forte), onsets}
Requer opencv/numpy (+ librosa para musica). Rode com o python do venv do rea-video.
"""
import json, sys, os, argparse
import cv2, numpy as np


def read_frames(path, end):
    cap = cv2.VideoCapture(path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30
    w, h = int(cap.get(3)), int(cap.get(4))
    frames = []
    while True:
        ok, f = cap.read()
        if not ok or len(frames) / fps > end:
            break
        frames.append(cv2.resize(f, (192, 192), interpolation=cv2.INTER_AREA))
    return frames, fps, w, h


def detect_cuts(frames):
    grays = [cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32) for f in frames]
    hists = []
    for g in grays:
        hh = cv2.calcHist([g.astype(np.uint8)], [0], None, [32], [0, 256]).ravel()
        hists.append(hh / max(1, hh.sum()))
    d = np.array([0.0] + [np.abs(grays[i] - grays[i - 1]).mean() for i in range(1, len(grays))])
    hd = np.array([0.0] + [np.abs(hists[i] - hists[i - 1]).sum() for i in range(1, len(hists))])
    med = np.median(d[1:]) if len(d) > 1 else 1
    def local_peak(i, r=12):
        # pico isolado contra a vizinhanca: pega corte dentro da mesma cena/luz em ref com muito movimento
        w = np.concatenate([d[max(1, i - r):i], d[i + 1:i + 1 + r]])
        if not (len(w) > 4 and d[i] > max(28, 3.5 * float(np.median(w))) and d[i] > 1.6 * max(d[i - 1], d[min(i + 1, len(d) - 1)])):
            return False
        # texto grande no centro (title card) muda o meio do quadro; corte muda tambem as laterais
        e = np.abs(grays[i] - grays[i - 1])
        edge = np.concatenate([e[:, :40].ravel(), e[:, -40:].ravel()]).mean()
        return edge >= 0.8 * d[i]
    cand = [i for i in range(1, len(d)) if (d[i] > max(18, 5 * med) and hd[i] > 0.25) or d[i] > max(30, 7 * med)
            or local_peak(i)]
    # flash: frame i muito diferente de i-1 E de i+1, mas i-1 ~ i+1  -> efeito de 1 frame, nao corte
    flashes, cuts = [], []
    for i in cand:
        if 0 < i < len(grays) - 1:
            around = np.abs(grays[i + 1] - grays[i - 1]).mean()
            if around < d[i] * 0.45:
                flashes.append(i)
                continue
        cuts.append(i)
    # cortes colados (efeito de 1-2 frames depois de um corte) viram um so; o frame seguinte ao flash pode ser corte real
    merged = []
    for c in cuts:
        if merged and c - merged[-1] <= 2:
            continue
        merged.append(c)
    return merged, flashes, d


def shot_stats(frames, a, b):
    idx = list(range(a + 1, max(a + 2, b - 1), max(1, (b - a) // 12)))
    idx = [k for k in idx if k < len(frames)] or [min(a, len(frames) - 1)]
    px = np.concatenate([frames[k].reshape(-1, 3) for k in idx]).astype(np.float32)
    lum = 0.114 * px[:, 0] + 0.587 * px[:, 1] + 0.299 * px[:, 2]
    hsv = cv2.cvtColor(px.reshape(-1, 1, 3).astype(np.uint8), cv2.COLOR_BGR2HSV).reshape(-1, 3)
    q = np.percentile(lum, [1, 5, 25, 50, 75, 90, 95, 99])
    bgr = px.mean(0)
    return {
        "luma": {f"p{p}": round(float(v), 1) for p, v in zip([1, 5, 25, 50, 75, 90, 95, 99], q)},
        "black_frac": round(float((lum < 12).mean()), 3),
        "clip_frac": round(float((lum > 245).mean()), 3),
        "sat": round(float(hsv[:, 1].mean()), 1),
        "rgb": [round(float(bgr[2])), round(float(bgr[1])), round(float(bgr[0]))],
    }


def camera(frames, a, b):
    if b - a < 6:
        return None
    gs = [cv2.cvtColor(frames[k], cv2.COLOR_BGR2GRAY).astype(np.float32) / 255 for k in range(a + 1, b - 1)]
    ref = gs[0]
    ys, last = [], None
    for k in range(0, len(gs), max(1, len(gs) // 8)):
        W = np.eye(2, 3, dtype=np.float32)
        try:
            _, W = cv2.findTransformECC(ref, gs[k], W, cv2.MOTION_AFFINE,
                                        (cv2.TERM_CRITERIA_EPS | cv2.TERM_CRITERIA_COUNT, 80, 1e-5), None, 5)
            s = float(np.sqrt(abs(np.linalg.det(W[:, :2]))))
            scale_px = 1080 / 192
            last = {"scale_end": round(s, 4), "tx_end": round(float(W[0, 2]) * scale_px), "ty_end": round(float(W[1, 2]) * scale_px)}
            ys.append(float(W[1, 2]) * scale_px)
        except cv2.error:
            pass
    if last is None:
        return None
    last["jitter_y"] = round(float(np.std(ys)), 1) if ys else 0
    return last


def music_info(path, end):
    try:
        import librosa
    except ImportError:
        return None
    y, sr = librosa.load(path, sr=22050, duration=end + 0.5)
    tempo, beats = librosa.beat.beat_track(y=y, sr=sr, units="time")
    env = librosa.onset.onset_strength(y=y, sr=sr)
    ts = librosa.times_like(env, sr=sr)
    top = np.argsort(env)[-6:]
    on = librosa.onset.onset_detect(y=y, sr=sr, units="time")
    period = float(np.median(np.diff(beats))) if len(beats) > 2 else None
    return {"beat_s": round(period, 3) if period else None, "beats": [round(float(b), 2) for b in beats],
            "strong_onsets": sorted(round(float(ts[k]), 3) for k in top),
            "onsets": [round(float(o), 2) for o in on]}


def voice_from_rea(rea_dir):
    p = os.path.join(rea_dir, "breakdown.json")
    if not os.path.exists(p):
        return None
    d = json.load(open(p, encoding="utf-8"))
    out = []

    def walk(x):
        if isinstance(x, dict):
            if "text" in x and any(k in x for k in ("start", "t0")) and any(k in x for k in ("end", "t1")):
                out.append({"text": x["text"], "start": x.get("start", x.get("t0")), "end": x.get("end", x.get("t1")),
                            "speaker": x.get("speaker")})
            for v in x.values():
                walk(v)
        elif isinstance(x, list):
            for v in x:
                walk(v)
    for key in ("asr", "script", "speech", "transcript", "lines"):
        if key in d:
            walk(d[key])
    if not out:
        walk(d.get("audio", {}))
    seen, uniq = set(), []
    for v in out:
        k = (v["text"], v["start"])
        if k not in seen and isinstance(v["start"], (int, float)):
            seen.add(k)
            uniq.append(v)
    return sorted(uniq, key=lambda v: v["start"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ref")
    ap.add_argument("out")
    ap.add_argument("--end", type=float, default=1e9)
    ap.add_argument("--rea")
    a = ap.parse_args()
    frames, fps, w, h = read_frames(a.ref, a.end)
    n = len(frames)
    cuts, flashes, _ = detect_cuts(frames)
    bounds = [0] + cuts + [n]
    shots = []
    for s, e in zip(bounds[:-1], bounds[1:]):
        if e - s < 1:
            continue
        st = shot_stats(frames, s, e)
        st.update({"from": s, "to": e, "dur_s": round((e - s) / fps, 3), "camera": camera(frames, s, e)})
        shots.append(st)
    res = {"ref": a.ref, "probe": {"fps": fps, "width": w, "height": h, "frames": n, "duration": round(n / fps, 3)},
           "cuts": cuts, "flashes": flashes, "shots": shots}
    stems = os.path.join(a.rea, "stems", "music.wav") if a.rea else None
    res["music"] = music_info(stems if stems and os.path.exists(stems) else a.ref, min(a.end, n / fps))
    if a.rea:
        res["voice"] = voice_from_rea(a.rea)
    json.dump(res, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"{len(cuts)} cortes, {len(flashes)} flashes, {len(shots)} planos -> {a.out}")


if __name__ == "__main__":
    main()
