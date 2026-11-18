"""
Refaz em alta resolucao os clipes do banco que vieram de fonte baixa (bug do client android = 360p).
Mesmo trecho (start/duration) da mesma fonte -> planos e falas ja catalogados continuam validos.

uso: python scripts/requalify_clips.py <canal> [--min 720] [--dry]
Backup dos arquivos antigos: out/clips_lowres_backup/<canal>/
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import yt_pace  # noqa: E402

CLIENTS = ["default", "web_safari"]   # igual ao footage.rs: no maximo 2 tentativas por video
FMT = "bv*[height>=1080][height<=2160]+ba/bv*[height>=720][height<=2160]+ba/b[height>=720]"


def download(url, dst):
    for c in CLIENTS:
        yt_pace.reserve(yt_pace.WEIGHT_DOWNLOAD)     # Blocked/Exhausted sobe: melhor parar o lote do que insistir
        r = subprocess.run(["yt-dlp", url, "--no-playlist", "-f", FMT, "--merge-output-format", "mp4", "-o", dst,
                            "--quiet", "--no-progress", "--extractor-args", f"youtube:player_client={c}",
                            *yt_pace.SLEEP_ARGS, *yt_pace.cookie_args()],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        if r.returncode == 0 and os.path.exists(dst):
            return True
        if yt_pace.is_block(r.stderr):
            yt_pace.mark_blocked()
            raise yt_pace.Blocked("YouTube bloqueou o IP; lote interrompido")
    return False


def height(p):
    r = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=height",
                        "-of", "csv=p=0", p], capture_output=True, text=True, encoding="utf-8", errors="replace")
    try:
        return int(r.stdout.strip().splitlines()[0])
    except Exception:
        return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("channel")
    ap.add_argument("--min", type=int, default=720)
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--force", action="store_true", help="refaz todos (ex.: mudou o aspecto do canal)")
    a = ap.parse_args()
    kdir = os.path.join("channels", a.channel, "knowledge")
    bank_p = os.path.join(kdir, "clips.json")
    bank = json.load(open(bank_p, encoding="utf-8"))
    aspect = json.load(open(os.path.join(kdir, "preset.json"), encoding="utf-8")).get("aspect", "9:16")
    aw, ah = {"1:1": (1, 1), "4:5": (4, 5), "4:3": (4, 3)}.get(aspect, (9, 16))
    vf = f"crop='min(iw,ih*{aw}/{ah})':'min(ih,iw*{ah}/{aw})',scale=1080:{1080 * ah // aw}:flags=lanczos,fps=30"
    todo = {}
    for c in bank:
        if 0 < c.get("src_height", 0) < a.min or a.force:
            todo.setdefault(c["source"], []).append(c)
    print(f"{sum(len(v) for v in todo.values())} clipes de {len(todo)} fontes abaixo de {a.min}p")
    if a.dry:
        return
    bdir = os.path.join("out", "clips_lowres_backup", a.channel)
    os.makedirs(bdir, exist_ok=True)
    tmpd = tempfile.mkdtemp()
    for i, (url, clips) in enumerate(todo.items()):
        src = os.path.join(tmpd, f"src{i}.mp4")
        if not download(url, src):
            print(f"[falhou] {url}: sem formato >= {a.min}p")
            continue
        h = height(src)
        if h < a.min:
            print(f"[falhou] {url}: so {h}p")
            continue
        for c in clips:
            dst = os.path.join("video", "public", c["file"])
            if os.path.exists(dst):
                shutil.copy2(dst, os.path.join(bdir, os.path.basename(dst)))
            tmp_out = dst + ".hq.mp4"
            r = subprocess.run(["ffmpeg", "-y", "-v", "error", "-ss", f"{c['start']:.3f}", "-t", f"{c['duration']:.3f}",
                                "-i", src, "-vf", vf, "-c:a", "aac", "-b:a", "160k", "-ac", "2",
                                "-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p", tmp_out])
            if r.returncode == 0 and os.path.exists(tmp_out):
                os.replace(tmp_out, dst)
                c["src_height"] = h
                c.pop("block", None)
                print(f"ok {c['file']} <- {h}p")
        os.remove(src)
        json.dump(bank, open(bank_p, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
    print("fim")


if __name__ == "__main__":
    main()
