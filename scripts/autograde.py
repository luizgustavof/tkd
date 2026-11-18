"""
Calibra o levels de cada plano casando a distribuicao de luma da FONTE com a do plano equivalente da REF.

uso: python scripts/autograde.py PLAN.json MEASURE.json PUBLIC_DIR [--saturate 0.15]
PLAN.json: {"shots": [{"src": "canal/clips/x.mp4", "startAt": 1.2, "rate": 0.5, "freeze": false,
                        "from": 0, "to": 127, "ref_shot": 0, ...}], "fps": 30}
Escreve shot["grade"] = {black, white, gamma, saturate, tint:[r,g,b], sharpen} no proprio PLAN.json.

Regra (mesma que fechou o experimento 01):
  black  = quantil da fonte igual a fracao de pretos (<12) da ref
  white  = quantil da fonte igual a 1 - fracao de altas estouradas (>245) da ref
  gamma  = leva a mediana (ou p75, se a mediana da ref ja e preto) da fonte ao valor da ref
  tint   = proporcao R:G:B dos medios da ref (normalizada no maior canal)
"""
import json, sys, math, argparse, os
import cv2, numpy as np


def src_luma(path, start, dur_src, freeze):
    cap = cv2.VideoCapture(path)
    cap.set(cv2.CAP_PROP_POS_MSEC, start * 1000)
    vals = []
    end = start + (0.05 if freeze else max(0.05, dur_src))
    while cap.get(cv2.CAP_PROP_POS_MSEC) <= end * 1000:
        ok, f = cap.read()
        if not ok:
            break
        f = cv2.resize(f, (160, 160)).reshape(-1, 3).astype(np.float32)
        vals.append(0.114 * f[:, 0] + 0.587 * f[:, 1] + 0.299 * f[:, 2])
    if not vals:
        raise SystemExit(f"sem frames em {path} @ {start}")
    return np.concatenate(vals)


def solve(lum, ref, sat_default):
    bf = min(0.95, max(0.0, ref["black_frac"]))
    cf = min(0.5, max(0.003, ref["clip_frac"]))
    black = float(np.quantile(lum, bf))
    white = float(np.quantile(lum, 1 - cf))
    if white - black < 12:
        white = black + 12
    key, q = ("p50", 0.5) if ref["luma"]["p50"] >= 12 else ("p75", 0.75)
    target = max(1.0, ref["luma"][key]) / 255
    v = (float(np.quantile(lum, q)) - black) / (white - black)
    v = min(0.98, max(0.02, v))
    gamma = math.log(target) / math.log(v) if target < 0.999 else 1.0
    gamma = min(3.5, max(0.5, gamma))
    r, g, b = ref["rgb"]
    m = max(r, g, b, 1)
    tint = [round(r / m, 3), round(g / m, 3), round(b / m, 3)]
    return {"black": round(black, 1), "white": round(white, 1), "gamma": round(gamma, 3),
            "saturate": sat_default, "tint": tint, "sharpen": 0.3}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("plan")
    ap.add_argument("measure")
    ap.add_argument("public_dir")
    ap.add_argument("--saturate", type=float, default=0.15)
    a = ap.parse_args()
    plan = json.load(open(a.plan, encoding="utf-8"))
    meas = json.load(open(a.measure, encoding="utf-8"))
    fps = plan.get("fps", 30)
    if plan.get("autograde") is False:  # canal que nao trata a cor do footage (ex.: jane)
        print("autograde desligado no plano")
        return
    for i, s in enumerate(plan["shots"]):
        if s.get("grade_lock"):
            continue
        ri = s.get("ref_shot", i)
        ref = meas["shots"][min(ri, len(meas["shots"]) - 1)]
        if s.get("fx") == "invert":
            # o plano sera invertido depois do grade: casar com o negativo da ref
            L = ref["luma"]
            ref = {**ref, "black_frac": ref["clip_frac"], "clip_frac": ref["black_frac"],
                   "luma": {"p25": 255 - L["p75"], "p50": 255 - L["p50"], "p75": 255 - L["p25"]},
                   "rgb": [255 - c for c in ref["rgb"]]}
        dur_src = (s["to"] - s["from"]) / fps * s.get("rate", 1)
        lum = src_luma(os.path.join(a.public_dir, s["src"]), s.get("startAt", 0), dur_src, s.get("freeze", False))
        s["grade"] = solve(lum, ref, s.get("saturate", plan.get("saturate", a.saturate)))
        print(f"shot {i} <- ref {ri}: {s['grade']}")
    json.dump(plan, open(a.plan, "w", encoding="utf-8"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
