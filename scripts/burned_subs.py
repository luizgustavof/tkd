"""
Legenda queimada na imagem (fansub, "English Sub" do YouTube): OCR na faixa de baixo de frames espalhados.

uso (python do rea-video, tem rapidocr + opencv):
  python scripts/burned_subs.py source MIDIA.mp4               -> JSON {has_subs, frac, top, texts} no stdout
  python scripts/burned_subs.py bank channels/<canal>/knowledge/clips.json [--force]
                                                             -> clipe com legenda: usable=false + tag

- so a faixa de baixo (35% do quadro) e so texto centralizado: placa/onomatopeia no meio da cena nao conta
- `top` = fracao da altura onde a legenda mais alta comeca (para recortar acima dela)
- OCR so na CPU (nada de DirectML: o driver AMD deu BSOD)
"""
import argparse, json, os, re, sys

import cv2
import numpy as np

BAND = 0.35          # faixa de baixo examinada
MIN_SCORE = 0.6      # confianca minima do OCR por caixa
LETTERS = re.compile(r"[A-Za-zÀ-ÿ]")

_OCR = None


def ocr():
    global _OCR
    if _OCR is None:
        from rapidocr_onnxruntime import RapidOCR
        _OCR = RapidOCR(det_use_cuda=False, cls_use_cuda=False, rec_use_cuda=False, det_use_dml=False,
                        cls_use_dml=False, rec_use_dml=False)
    return _OCR


def frames(path, n):
    cap = cv2.VideoCapture(path)
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0)
    out = []
    if total <= 0:
        cap.release()
        return out
    # evita as pontas (intro/outro do canal) em fontes longas
    lo, hi = (0.05, 0.95) if total > 30 * 120 else (0.0, 1.0)
    for k in range(n):
        idx = int(total * (lo + (hi - lo) * (k + 0.5) / n))
        cap.set(cv2.CAP_PROP_POS_FRAMES, idx)
        ok, fr = cap.read()
        if ok:
            out.append(fr)
    cap.release()
    return out


def text_in_band(fr):
    """Caixas de texto (topo em fracao da altura, texto) na faixa de baixo, centralizadas."""
    h, w = fr.shape[:2]
    y0 = int(h * (1 - BAND))
    band = fr[y0:]
    # OCR em ~960 px de largura: legenda continua legivel e fica 4x mais rapido que 4K
    sc = min(1.0, 960 / w)
    if sc < 1:
        band = cv2.resize(band, (int(w * sc), int(band.shape[0] * sc)), interpolation=cv2.INTER_AREA)
    res, _ = ocr()(band)
    out = []
    for box, txt, score in res or []:
        if float(score) < MIN_SCORE or len(LETTERS.findall(txt)) < 3:
            continue
        xs = [p[0] / sc for p in box]
        ys = [p[1] / sc for p in box]
        cx = (min(xs) + max(xs)) / 2
        if abs(cx - w / 2) > w * 0.3:  # legenda e centralizada
            continue
        out.append(((y0 + min(ys)) / h, txt))
    return out


def scan(path, n):
    fs = frames(path, n)
    hits, tops, texts = 0, [], []
    for fr in fs:
        t = text_in_band(fr)
        if t:
            hits += 1
            tops.append(min(y for y, _ in t))
            texts.append(" ".join(x for _, x in t))
    frac = hits / max(1, len(fs))
    return {"frames": len(fs), "hits": hits, "frac": round(frac, 3),
            "top": round(float(np.percentile(tops, 10)), 3) if tops else None, "texts": texts[:6]}


def cmd_source(a):
    r = scan(a.path, a.frames)
    # fonte: legenda em >= 15% dos frames (dialogo nao e continuo); 2 hits no minimo contra placa isolada
    r["has_subs"] = r["hits"] >= 2 and r["frac"] >= 0.15
    json.dump(r, sys.stdout, ensure_ascii=False)


def cmd_bank(a):
    bank = json.load(open(a.path, encoding="utf-8"))
    pub = os.path.join("video", "public")
    n = 0
    for c in bank:
        if "burned_subs" in c and not a.force:
            continue
        p = os.path.join(pub, c["file"])
        if not os.path.exists(p):
            continue
        r = scan(p, a.frames)
        # clipe curto: 2 frames com texto ja e legenda (aparece/some com a fala)
        has = r["hits"] >= 2
        c["burned_subs"] = has
        if has:
            c["usable"] = False
            if "legenda queimada" not in c.get("tags", []):
                c.setdefault("tags", []).append("legenda queimada")
            n += 1
            print(f"{c['file']}: legenda ({r['hits']}/{r['frames']}) {r['texts'][:2]}")
    json.dump(bank, open(a.path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
    print(f"{n} clipes com legenda queimada marcados como inutilizaveis")


def main():
    ap = argparse.ArgumentParser()
    sp = ap.add_subparsers(dest="cmd", required=True)
    s = sp.add_parser("source")
    s.add_argument("path")
    s.add_argument("--frames", type=int, default=40)
    s.set_defaults(fn=cmd_source)
    b = sp.add_parser("bank")
    b.add_argument("path")
    b.add_argument("--frames", type=int, default=8)
    b.add_argument("--force", action="store_true")
    b.set_defaults(fn=cmd_bank)
    a = ap.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    a.fn(a)


if __name__ == "__main__":
    main()
