"""
Da vida a uma abertura parada: movimento em toda batida + flash cuts de 4 frames do personagem.

uso (python do rea-video, precisa de librosa):
  python scripts/intro_energy.py PROPS.json OUT.json PUBLIC_DIR --channel SLUG --until 207 --inserts SRC@START,SRC@START,...
                                 [--at 3.67,4.6,5.53] [--layers-amount 1.5]
- batidas reais da trilha ate --until: pulse em todas; shake+rgb nas fortes
- --inserts: cada SRC@START vira um corte de 4 frames na batida de --at (segundos), voltando ao plano
  que estava tocando (que continua de onde estaria, camera interpolada por pedaco)
"""
import argparse, copy, json, os
import librosa, numpy as np


def beats(path, until_s, fps):
    y, sr = librosa.load(path, sr=22050, duration=until_s + 0.5)
    env = librosa.onset.onset_strength(y=y, sr=sr)
    _, b = librosa.beat.beat_track(onset_envelope=env, sr=sr)
    strong = np.quantile(env[b], 0.5) if len(b) else 0
    return [(int(round(librosa.frames_to_time(f, sr=sr) * fps)), bool(env[f] >= strong)) for f in b]


def lerp_cam(cam, k):
    a, b = cam.get("from", {}), cam.get("to", cam.get("from", {}))
    return {key: a.get(key, d) + (b.get(key, a.get(key, d)) - a.get(key, d)) * k
            for key, d in (("s", 1), ("x", 0), ("y", 0))}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("props"); ap.add_argument("out"); ap.add_argument("public_dir")
    ap.add_argument("--until", type=int, required=True)
    ap.add_argument("--inserts", default="")
    ap.add_argument("--channel", required=True, help="slug do canal (pasta em PUBLIC_DIR com os clipes)")
    ap.add_argument("--at", default="3.67,4.6,5.53")
    ap.add_argument("--layers-amount", type=float, default=1.5)
    a = ap.parse_args()
    p = json.load(open(a.props, encoding="utf-8"))
    fps = p["fps"]
    ev = [e for e in p["motion"] if not (e["f"] < a.until and e["kind"] == "pulse")]
    bs = beats(os.path.join(a.public_dir, p["audio"]["music"]), a.until / fps, fps)
    for f, strong in bs:
        if f >= a.until - 3:
            continue
        ev.append({"f": f, "kind": "pulse", "amount": 1.045, "dur": 7})
        if strong:
            ev.append({"f": f, "kind": "shake", "amount": 5, "dur": 5})
            ev.append({"f": f, "kind": "rgb", "amount": 3, "dur": 4})
    for s in p["shots"]:
        if s["from"] < a.until and s.get("layers"):
            s["layers"]["amount"] = a.layers_amount

    # flash cuts
    shots = p["shots"]
    for spec, t in zip([x for x in a.inserts.split(",") if x], [float(x) for x in a.at.split(",")]):
        src, start = spec.split("@")
        f = int(round(t * fps))
        i = next(i for i, s in enumerate(shots) if s["from"] < f < s["to"] - 4)
        s = shots[i]
        n = s["to"] - s["from"]
        rate = s.get("rate", 1) or 1
        left, right = copy.deepcopy(s), copy.deepcopy(s)
        left["to"] = f
        right["from"] = f + 4
        right["startAt"] = round((s.get("startAt", 0) or 0) + (right["from"] - s["from"]) / fps * rate, 3)
        if s.get("camera"):
            k1, k2 = (f - s["from"]) / n, (right["from"] - s["from"]) / n
            left["camera"] = {"from": lerp_cam(s["camera"], 0), "to": lerp_cam(s["camera"], k1), "easing": "linear"}
            right["camera"] = {"from": lerp_cam(s["camera"], k2), "to": lerp_cam(s["camera"], 1), "easing": "linear"}
        donor = next((x for x in shots if x["src"].endswith(src)), None)
        ins = {"from": f, "to": f + 4, "src": f"{a.channel}/clips/{src}", "startAt": float(start),
               "grade": (donor or {}).get("grade"), "note": "flash cut da abertura"}
        shots[i:i + 1] = [left, ins, right]
        ev += [{"f": f, "kind": "punch", "amount": 1.15, "dur": 4}, {"f": f, "kind": "flash", "amount": 0.45, "dur": 3}]
        print(f"flash cut {src}@{start} em {f} (dentro do plano {s['from']}-{s['to']})")

    ev.sort(key=lambda e: (e["f"], e["kind"]))
    p["motion"] = ev
    json.dump(p, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"abertura: {len(bs)} batidas ate {a.until}, {sum(1 for _, s in bs if s)} fortes")


if __name__ == "__main__":
    main()
