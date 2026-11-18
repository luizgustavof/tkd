"""
Voz = falas REAIS do footage (sem TTS). Gera voice.json (mesmo formato do voice_lines.py) a partir dos
planos do plano que tocam o audio do proprio clipe.

uso: python scripts/clip_voice.py PLAN.json CLIPS.json OUT_VOICE.json [--lang en|pt]
Em PLAN.json, um plano com fala:
  {"src": "canal/clips/x.mp4", "from": 0, "to": 90, "startAt": 0.2, "volume": 1,
   "speech": {"line": 0,                      # indice em clips.json[src].speech
              "translation": "...",           # SO se a fala nao esta no idioma do edit (ex.: japones -> ingles)
              "captions": [{"words": [0,1,2], "kind": "caption"|"title", "end_max": 120}],   # opcional
              "split": {"remember": ["REMEM-"]}}}                                           # opcional
Tempo de cada palavra no video = from/fps + (t_no_clipe - startAt) / rate.
Fala traduzida: as palavras da traducao sao distribuidas so nos trechos com voz da fala original (proporcional
ao tamanho); silencio no meio da fala fica sem legenda nova.
"""
import argparse, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from voice_lines import build_captions  # noqa: E402

PUNCT = re.compile(r"^[^\w']+|[^\w'\-]+$", re.UNICODE)


def clean(w):
    return PUNCT.sub("", w.strip())


VOICE_GAP = 0.35  # s sem palavra = silencio (a legenda traduzida nao anda nele)


def voiced(words):
    """Trechos com voz [(s, e)] a partir das palavras do whisper (tempo no clipe)."""
    out = []
    for w in words:
        if out and w["s"] - out[-1][1] <= VOICE_GAP:
            out[-1][1] = max(out[-1][1], w["e"])
        else:
            out.append([w["s"], w["e"]])
    return out


def spread(text, spans):
    """Palavras da traducao distribuidas SO nos trechos com voz (proporcional ao tamanho da palavra).
    Antes era do inicio ao fim da fala original: um silencio no meio fazia a legenda andar com a boca fechada."""
    toks = [t for t in text.split() if clean(t)]
    total_t = sum(e - s for s, e in spans) or 1e-6
    total = sum(len(clean(t)) + 1 for t in toks) or 1
    out, pos = [], 0.0  # pos = segundos de voz ja usados

    def at(x):  # segundos de voz -> tempo real
        for s, e in spans:
            if x <= e - s + 1e-9:
                return s + x
            x -= e - s
        return spans[-1][1]

    for tok in toks:
        d = total_t * (len(clean(tok)) + 1) / total
        s = at(pos)
        e = at(pos + d * 0.92)
        # palavra nao atravessa silencio: termina no fim do trecho em que comecou
        span_end = next((b for a2, b in spans if a2 - 1e-9 <= s <= b + 1e-9), e)
        out.append({"w": clean(tok), "raw": tok, "s": s, "e": min(e, span_end)})
        pos += d
    return out


def resolve_words(captions, ws, lid):
    """`words` pode vir como texto ("fun") em vez de indice: o agente errava a contagem (Toji 18/11: indice 5 =
    "the", queria "fun"). Cada palavra e procurada a partir da ultima achada, na ordem da fala."""
    out, pos = [], 0
    for cap in captions:
        idx = []
        for w in cap.get("words") or []:
            if isinstance(w, int):
                idx.append(w)
                pos = w + 1
                continue
            key = clean(str(w)).lower()
            hit = next((k for k in range(pos, len(ws)) if ws[k]["w"].lower() == key), None)
            if hit is None:
                hit = next((k for k in range(len(ws)) if ws[k]["w"].lower() == key), None)
            if hit is None:
                print(f"[aviso] {lid}: palavra '{w}' da legenda nao esta na fala", file=sys.stderr)
                continue
            idx.append(hit)
            pos = hit + 1
        out.append({**cap, "words": idx})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("plan")
    ap.add_argument("clips")
    ap.add_argument("out")
    ap.add_argument("--lang", default="en")
    a = ap.parse_args()
    plan = json.load(open(a.plan, encoding="utf-8"))
    bank = {c["file"]: c for c in json.load(open(a.clips, encoding="utf-8"))}
    fps = plan.get("fps", 30)
    result = {"lines": [], "captions": []}
    items = [(i, s, s.get("speech")) for i, s in enumerate(plan["shots"]) if s.get("speech")]
    # voiceClips: fala tocando por cima de quaisquer planos; vira um "plano virtual" so de audio
    for j, vc in enumerate(plan.get("voiceClips") or []):
        clip = bank.get(vc["src"]) or {}
        lines = clip.get("speech") or []
        k = vc.get("line", 0)
        if k >= len(lines):
            print(f"[aviso] voiceClip {j}: {vc['src']} nao tem fala {k}", file=sys.stderr)
            continue
        ln = lines[k]
        pre = 0.15
        start_at = max(0.0, ln["start"] - pre)
        at = vc["at"]  # frame em que a fala comeca a soar
        f0 = max(0, at - round(pre * fps))
        to = f0 + round((ln["end"] - start_at + 0.3) * fps)
        vc.update({"from": f0, "to": to, "startAt": round(start_at, 3)})
        items.append((f"v{j}", {"src": vc["src"], "from": f0, "to": to, "startAt": start_at, "rate": 1},
                      {k2: v for k2, v in vc.items() if k2 in ("line", "translation", "captions", "split")}))
    json.dump(plan, open(a.plan, "w", encoding="utf-8"), indent=1, ensure_ascii=False)  # grava from/to/startAt dos voiceClips
    for i, s, sp in items:
        clip = bank.get(s["src"])
        lines = (clip or {}).get("speech") or []
        k = sp.get("line", 0)
        if k >= len(lines):
            print(f"[aviso] shot {i}: {s['src']} nao tem fala {k}", file=sys.stderr)
            continue
        ln = lines[k]
        rate = s.get("rate", 1) or 1
        if abs(rate - 1) > 1e-3:
            print(f"[aviso] shot {i}: rate {rate} altera o tom da voz; use 1 em plano com fala", file=sys.stderr)
        t0 = s["from"] / fps
        to_video = lambda t: t0 + (t - s.get("startAt", 0)) / rate
        if sp.get("translation") or (ln.get("lang") and ln["lang"] != a.lang and not a.lang.startswith(ln["lang"])):
            text = sp.get("translation")
            if not text:
                print(f"[aviso] shot {i}: fala em {ln.get('lang')} sem traducao para {a.lang}; legenda no original",
                      file=sys.stderr)
                text = ln["text"]
            ws = spread(text, [(to_video(a), to_video(b)) for a, b in voiced(ln.get("words") or [{"s": ln["start"], "e": ln["end"]}])])
        else:
            ws = [{"w": clean(w["w"]), "raw": w["w"], "s": to_video(w["s"]), "e": to_video(w["e"])}
                  for w in ln["words"] if clean(w["w"])]
        end_shot = s["to"] / fps
        ws = [w for w in ws if w["s"] < end_shot - 0.02]  # palavra que cai depois do corte nao aparece
        if not ws:
            continue
        lid = f"s{i}"
        result["lines"].append({"id": lid, "text": " ".join(w["w"] for w in ws), "lang": ln.get("lang"),
                                "src": s["src"], "words": ws})
        spec = dict(sp)
        if spec.get("captions"):
            spec["captions"] = resolve_words(spec["captions"], ws, lid)
        caps = build_captions(lid, ws, spec, fps, start_index=len(result["captions"]))
        for c in caps:  # legenda nunca passa do fim do plano que fala, salvo se pedido
            if "end_max" not in str(sp.get("captions")):
                c["end"] = min(c["end"], s["to"] + 6)
        result["captions"] += caps
        print(f"{lid} [{ln.get('lang')}] " + " ".join(f"{w['w']}@{w['s']:.2f}" for w in ws))
    json.dump(result, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"{len(result['lines'])} falas, {len(result['captions'])} legendas -> {a.out}")


if __name__ == "__main__":
    main()
