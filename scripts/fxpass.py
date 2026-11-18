"""
Passe de efeitos "AE" sobre props prontas do recreate (teste; sem agente, sem tokens).

uso: python scripts/fxpass.py PROPS.json OUT.json PUBLIC_DIR [--slow N1,N2,...] [--rate 0.35]

Adiciona:
  look.deepGlow           Deep Glow no frame inteiro (soma com o look.glow simples, que levanta as sombras)
  motion glow/zoomblur    nos frames de frameFx (white/invert) e nos cortes mais fortes (punch+shake)
  motion warp             na saida da abertura e na entrada do ultimo plano
  motion rays             no plano final (fundo escuro com pontos claros)
  slow-mo interpolado     planos --slow (indices) ganham rate baixo e um clipe interpolado por fluxo
                          optico (ffmpeg minterpolate, so CPU), para a camera lenta ficar lisa
"""
import argparse, json, math, os, subprocess


def interp_clip(public_dir, src, start, dur, rate, fps):
    target = int(math.ceil(fps / rate / 10.0) * 10)
    base = os.path.splitext(os.path.basename(src))[0]
    rel = f"{src.split('/')[0]}/interp/{base}-{start:.2f}-{target}.mp4"
    out = os.path.join(public_dir, rel)
    if not os.path.exists(out):
        os.makedirs(os.path.dirname(out), exist_ok=True)
        vf = f"minterpolate=fps={target}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{start:.3f}", "-t", f"{dur:.3f}",
                        "-i", os.path.join(public_dir, src), "-vf", vf, "-an",
                        "-c:v", "libx264", "-crf", "14", "-preset", "medium", "-pix_fmt", "yuv420p", out], check=True)
    return rel, target


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("props")
    ap.add_argument("out")
    ap.add_argument("public_dir")
    ap.add_argument("--slow", default="")
    ap.add_argument("--rate", type=float, default=0.35)
    ap.add_argument("--layers", action="append", default=[],
                    help="IDX:pan|dolly:PREFIXO (PREFIXO-bg.png/-fg.png relativos a public, de parallax_layers.py)")
    ap.add_argument("--beat-fx", action="store_true", help="zoomblur+glow tambem nos onsets fortes (nao so nos gatilhos)")
    ap.add_argument("--hook", help="PREFIXO:FRAMES:START — abre com FRAMES de parallax e a abertura original entra em START s")
    a = ap.parse_args()
    p = json.load(open(a.props, encoding="utf-8"))
    fps = p["fps"]
    total = p["durationInFrames"]
    shots = p["shots"]
    ev = list(p.get("motion") or [])

    look = dict(p.get("look") or {})
    look["deepGlow"] = {"amount": 0.4, "threshold": 0.68, "radius": 1}
    p["look"] = look

    def add(f, kind, amount, dur, **kw):
        if 0 <= f < total:
            ev.append({"f": int(f), "kind": kind, "amount": amount, "dur": int(dur), **kw})

    # gatilhos de 1 frame: estouro de glow + zoom radial
    fx = sorted(int(k) for k in (p.get("frameFx") or {}))
    for f in fx:
        add(f + 1, "glow", 0.7, 12)
        add(f + 1, "zoomblur", 0.22, 9)

    # cortes mais fortes (punch que veio junto com shake = onset forte da musica), espacados.
    # Desligado por padrao: no teste do Toji o v4 leu esses pulsos como cortes a mais ("frames picotados")
    shaken = sorted({e["f"] for e in ev if e["kind"] == "shake"}) if a.beat_fx else []
    last = -999
    for f in shaken:
        if f - last >= 45 and all(abs(f - g) > 6 for g in fx):
            add(f, "zoomblur", 0.14, 7)
            add(f, "glow", 0.35, 8)
            last = f

    # warp nas viradas de bloco: saida da abertura e entrada do plano final
    if len(shots) > 1:
        add(shots[1]["from"] - 8, "warp", 70, 16)
        add(shots[-1]["from"] - 9, "warp", 60, 18)
    # raios de luz: abertura e final
    sl = shots[-1]
    add(sl["from"], "rays", 0.7, sl["to"] - sl["from"], x=0.5, y=0.4)

    # camera lenta interpolada
    for i in [int(x) for x in a.slow.split(",") if x.strip()]:
        s = shots[i]
        n = s["to"] - s["from"]
        dur = n / fps * a.rate + 0.4
        rel, target = interp_clip(a.public_dir, s["src"], s.get("startAt", 0) or 0, dur, a.rate, fps)
        s["note"] = f"slow-mo {a.rate} interpolado a {target} fps (era {s['src']} @ {s.get('startAt', 0)})"
        s["src"], s["startAt"], s["rate"] = rel, 0, a.rate
        print(f"slow-mo: plano {i} ({s['from']}-{s['to']}) -> {rel}")

    # parallax 2.5D: o plano vira duas camadas paradas que a camera separa
    for spec in a.layers:
        idx, mode, prefix = spec.split(":", 2)
        s = shots[int(idx)]
        s["layers"] = {"bg": f"{prefix}-bg.png", "fg": f"{prefix}-fg.png", "mode": mode, "amount": 1}
        s.pop("camera", None)
        print(f"parallax: plano {idx} ({s['from']}-{s['to']}) {mode}")

    # gancho: abre no rosto do personagem em parallax e so depois entra o plano de abertura original
    # (por ultimo: muda os indices dos planos)
    if a.hook:
        prefix, n, start = a.hook.split(":")
        n, s0 = int(n), shots[0]
        head = {k: v for k, v in s0.items() if k not in ("camera", "startAt", "ref_shot", "note")}
        head.update({"from": s0["from"], "to": s0["from"] + n, "note": "gancho: rosto em parallax",
                     "layers": {"bg": f"{prefix}-bg.png", "fg": f"{prefix}-fg.png", "mode": "pan", "amount": 0.8, "dir": -1}})
        s0["from"] += n
        s0["startAt"] = float(start)
        shots.insert(0, head)
        add(n, "punch", 1.12, 6)
        add(n, "zoomblur", 0.18, 8)
        print(f"gancho: 0-{n} parallax, abertura original agora {s0['from']}-{s0['to']} @ {start}s")

    ev.sort(key=lambda e: (e["f"], e["kind"]))
    p["motion"] = ev
    p["id"] = p.get("id", "recreate") + "-fx"
    json.dump(p, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    kinds = {}
    for e in ev:
        kinds[e["kind"]] = kinds.get(e["kind"], 0) + 1
    print(f"fxpass: {kinds} -> {a.out}")


if __name__ == "__main__":
    main()
