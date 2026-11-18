"""
Qualidade REAL de cada clipe do banco (antes de grao/grade do render).

uso: python scripts/clip_quality.py bank channels/<canal>/knowledge/clips.json [--force]
     python scripts/clip_quality.py file CLIP.mp4 [START DUR]
Campos gravados no clipe:
  src_height  altura do video de ORIGEM no YouTube (o recorte e sempre 1080; isso diz se foi ampliado)
  block       blocagem de compressao: razao das diferencas nas fronteiras de 8 px (>1.5 = macrobloco visivel)
Regra (docs/QUALITY_RULES.md): src_height >= 720 e block <= 1.5 para plano visivel > 0.5 s.
"""
import sys, json, os, subprocess
import cv2, numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import yt_pace  # noqa: E402


def block_of(img):
    g = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY).astype(np.float32)
    dx = np.abs(np.diff(g, axis=1))
    return float(dx[:, 7::8].mean() / max(np.delete(dx, np.s_[7::8], axis=1).mean(), 1e-3))


def clip_block(path, start=0.0, dur=None, n=5):
    cap = cv2.VideoCapture(path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 30
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT)) or 1
    a = int(start * fps)
    b = min(total - 1, int((start + dur) * fps)) if dur else total - 1
    vals = []
    for f in np.linspace(a, max(a, b), n).astype(int):
        cap.set(cv2.CAP_PROP_POS_FRAMES, int(f))
        ok, img = cap.read()
        if ok and img.mean() >= 10:
            vals.append(block_of(img))
    return round(float(np.median(vals)), 2) if vals else None


_H = {}


def source_height(url):
    """altura que o download usou (melhor formato ate 1080, como o tkd footage antigo baixava)."""
    if not url:
        return None
    if url not in _H:
        h = None
        for client in ("default", "web_safari"):   # igual ao footage.rs (android so entrega 360p)
            try:
                yt_pace.reserve(3)                  # pagina + player: orcamento compartilhado com o tkd
            except (yt_pace.Blocked, yt_pace.Exhausted) as e:
                print(f"source_height pulado: {e}", file=sys.stderr)
                break
            r = subprocess.run(["yt-dlp", url, "--no-playlist", "--skip-download", "-f", "bv*[height<=1080]/b[height<=1080]/best",
                                "--print", "%(height)s", "--extractor-args", f"youtube:player_client={client}",
                                "--sleep-requests", "1", *yt_pace.cookie_args()],
                               capture_output=True, text=True, encoding="utf-8", errors="replace")
            out = r.stdout.strip().splitlines()
            if r.returncode == 0 and out and out[-1].isdigit():
                h = int(out[-1])
                break
            if yt_pace.is_block(r.stderr):
                yt_pace.mark_blocked()
                break
        _H[url] = h
    return _H[url]


def main():
    mode = sys.argv[1]
    if mode == "file":
        st = float(sys.argv[3]) if len(sys.argv) > 3 else 0
        du = float(sys.argv[4]) if len(sys.argv) > 4 else None
        print(json.dumps({"block": clip_block(sys.argv[2], st, du)}))
        return
    path = sys.argv[2]
    force = "--force" in sys.argv
    bank = json.load(open(path, encoding="utf-8"))
    for c in bank:
        p = os.path.join("video", "public", c["file"])
        if not os.path.exists(p):
            continue
        if force or c.get("block") is None:
            c["block"] = clip_block(p)
        if force or not c.get("src_height"):
            h = source_height(c.get("source"))
            if h:
                c["src_height"] = h
        print(f"{c['file']}: src {c.get('src_height')}p block {c.get('block')}")
    json.dump(bank, open(path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)


if __name__ == "__main__":
    main()
