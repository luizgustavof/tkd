"""
Portao de qualidade do video final (regras em docs/QUALITY_RULES.md). Roda depois do render.

uso: python scripts/qa.py VIDEO.mp4 PROPS.json CLIPS.json OUT_QA.json [--lufs -12] [--character "Nome"]
Saida: {"pass": bool, "score": 0-100, "issues": [{"level": "erro"|"aviso", "what", "where"}], "metrics": {...}}
erro  = viola regra (nao publicar sem corrigir)
aviso = abaixo do ideal
"""
import argparse, json, re, subprocess
import cv2, numpy as np


def ffprobe(p):
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries",
                        "stream=codec_type,width,height,r_frame_rate,bit_rate:format=duration,bit_rate",
                        "-of", "json", p], capture_output=True, text=True, encoding="utf-8", errors="replace")
    return json.loads(r.stdout)


def loudness(p):
    r = subprocess.run(["ffmpeg", "-i", p, "-af", "ebur128=peak=true", "-f", "null", "-"], capture_output=True, text=True, encoding="utf-8", errors="replace")
    txt = r.stderr
    i = re.findall(r"I:\s+(-?[\d.]+) LUFS", txt)
    tp = re.findall(r"Peak:\s+(-?[\d.]+) dBFS", txt)
    return (float(i[-1]) if i else None), (float(tp[-1]) if tp else None)


def straight_band(lines):
    """px de preto puro colado na borda que termina num salto brusco para a imagem: tarja da fonte ou camera/shake
    alem do zoom (canal de edits, 17/11). Sombra de anime e irregular e nao passa no salto. 0 = sem faixa."""
    n = 0
    for v in lines:
        if v.max() < 10 and v.mean() < 3:
            n += 1
        else:
            break
    if n < 3 or n >= len(lines) - 1:
        return 0
    return n if (lines[n] > 25).mean() > 0.6 else 0


def edge_bands(img):
    """{lado: px} das faixas pretas retas, medidas em 540 px de largura (x2 = px no 1080)."""
    g = cv2.cvtColor(cv2.resize(img, (540, max(1, 540 * img.shape[0] // img.shape[1])), interpolation=cv2.INTER_AREA),
                     cv2.COLOR_BGR2GRAY).astype(np.float32)
    e = {"cima": straight_band(g), "baixo": straight_band(g[::-1]), "esq": straight_band(g.T), "dir": straight_band(g.T[::-1])}
    return {k: v * 2 for k, v in e.items() if v}


def frame_stats(p):
    cap = cv2.VideoCapture(p)
    lum, block, bands = [], [], []
    n = 0
    while True:
        ok, img = cap.read()
        if not ok:
            break
        b = edge_bands(img)
        if b:
            bands.append((n, b))
        small = cv2.resize(img, (270, 270), interpolation=cv2.INTER_AREA)
        g_small = cv2.cvtColor(small, cv2.COLOR_BGR2GRAY)
        # preto de verdade (startAt fora do clipe, buraco) e chapado: p99 ~0. Cena escura de edit (Hellsing a noite,
        # grade azul) tem media < 6 mas raio/contorno acesos (p99 14-91) e era reprovada como "frames pretos".
        lum.append(float(g_small.mean()) if np.percentile(g_small, 99) < 10 else 255.0)
        if n % 15 == 0:
            g = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY).astype(np.float32)
            dx = np.abs(np.diff(g, axis=1))
            block.append((n, float(dx[:, 7::8].mean() / max(np.delete(dx, np.s_[7::8], axis=1).mean(), 1e-3))))
        n += 1
    return np.array(lum), block, bands


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("video")
    ap.add_argument("props")
    ap.add_argument("clips")
    ap.add_argument("out")
    ap.add_argument("--lufs", type=float, default=-12.0)
    ap.add_argument("--character")
    a = ap.parse_args()
    props = json.load(open(a.props, encoding="utf-8"))
    bank = {c["file"]: c for c in json.load(open(a.clips, encoding="utf-8"))}
    fps = props.get("fps", 30)
    issues = []

    def add(level, what, where=""):
        issues.append({"level": level, "what": what, "where": where})

    # 1. formato
    pr = ffprobe(a.video)
    v = next((s for s in pr["streams"] if s["codec_type"] == "video"), {})
    au = next((s for s in pr["streams"] if s["codec_type"] == "audio"), None)
    if v.get("width", 0) < 1080:
        add("erro", f"largura {v.get('width')} < 1080")
    vbr = int(v.get("bit_rate") or pr["format"].get("bit_rate") or 0)
    if vbr and vbr < 6_000_000:
        add("aviso", f"bitrate de video {vbr / 1e6:.1f} Mbps (< 6): TikTok recomprime, comece alto")
    # 2. audio
    if au is None:
        add("erro" if (props.get("audio") or {}).get("music") or props.get("voiceClips") else "aviso", "video sem audio")
        lufs = tp = None
    else:
        lufs, tp = loudness(a.video)
        if lufs is not None and abs(lufs - a.lufs) > 1.5:
            add("erro", f"loudness {lufs:.1f} LUFS fora do alvo {a.lufs}±1.5")
        if tp is not None and tp > -0.5:
            add("aviso", f"true peak {tp:.1f} dBFS (> -0.5, pode clipar)")
    # 3. imagem: pretos indesejados e macrobloco no render
    lum, blocks, bands = frame_stats(a.video)
    # o endCard e texto da marca sobre fundo preto de proposito (canal de edits, 17/11: "pretos no frame 726" era ele)
    end_card = (props.get("endCard") or {}).get("from")
    black_until = int(end_card) if isinstance(end_card, (int, float)) and end_card > 0 else len(lum)
    run = 0
    for i, l in enumerate(lum[:black_until]):
        run = run + 1 if l < 6 else 0
        if run == 4:
            add("erro", "4+ frames pretos seguidos (corte interno da fonte ou startAt fora do clipe)", f"frame {i - 3}")
    # borda preta reta (tarja da fonte ou camera alem do zoom); o endCard e preto de proposito
    bands = [(f, b) for f, b in bands if f < black_until]
    if len(bands) >= 3:
        f0, b0 = bands[0]
        sides = ", ".join(f"{k} {v}px" for k, v in b0.items())
        add("erro", f"faixa preta na borda em {len(bands)} frames (tarja da fonte ou camera alem do zoom; 1o: {sides})",
            f"frame {f0}")
    for f, b in blocks:
        if b > 1.6:
            add("aviso", f"macrobloco visivel no render ({b:.2f})", f"frame {f}")
    # 4. footage por plano: resolucao de origem, blocagem, personagem
    name_parts = [p.lower() for p in (a.character or "").split() if len(p) >= 3]
    for i, s in enumerate(props["shots"]):
        c = bank.get(s["src"])
        dur = (s["to"] - s["from"]) / fps
        where = f"plano {i} ({s['from']}-{s['to']}) {s['src'].split('/')[-1]}"
        if c is None:
            add("erro", "clipe fora do banco", where)
            continue
        sh = c.get("src_height", 0)
        if 0 < sh < 720 and dur > 0.5:
            add("erro", f"fonte {sh}p ampliada para 1080 (minimo 720p em plano > 0.5 s)", where)
        if (c.get("block") or 0) > 1.5 and dur > 0.5:
            add("aviso", f"clipe com macrobloco ({c['block']})", where)
        if "so-audio" in c.get("tags", []) and dur > 0.2:
            add("erro", "clipe marcado so-audio usado como imagem (legenda queimada/logo)", where)
        if name_parts:
            hay = f"{c.get('desc', '')} {' '.join(c.get('tags', []))} {c.get('source_title', '')}".lower()
            if not any(p in hay for p in name_parts):
                add("aviso", f"plano sem o personagem `{a.character}` na descricao", where)
        # zoom digital alto em fonte fraca = imagem mole
        cam = (s.get("camera") or {}).get("to") or {}
        zoom = max(cam.get("s", 1), (s.get("camera") or {}).get("from", {}).get("s", 1)) * s.get("overscan", 1)
        if zoom > 1.25 and 0 < sh < 1440:
            add("aviso", f"zoom {zoom:.2f}x sobre fonte {sh}p", where)
    # 5. legendas: tempo minimo de leitura
    for c in props.get("captions") or []:
        st = c["steps"]
        if st and (c["end"] - st[-1]["f"]) < 6:
            add("aviso", "legenda completa fica < 0.2 s na tela", f"frame {st[-1]['f']}")

    errs = sum(1 for x in issues if x["level"] == "erro")
    warns = len(issues) - errs
    score = max(0, 100 - 15 * errs - 3 * warns)
    res = {"pass": errs == 0, "score": score, "issues": issues,
           "metrics": {"lufs": lufs, "true_peak": tp, "video_bitrate": vbr, "width": v.get("width"), "height": v.get("height")}}
    json.dump(res, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"QA {'PASSOU' if res['pass'] else 'REPROVADO'} nota {score} — {errs} erros, {warns} avisos")
    for x in issues:
        print(f"  [{x['level']}] {x['what']}  {x['where']}")


if __name__ == "__main__":
    main()
