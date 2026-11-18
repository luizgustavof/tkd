"""
Gera falas com ElevenLabs (with-timestamps), encaixa cada fala na janela de tempo da referencia
e devolve os passos de legenda (palavra a palavra, com silabas hifenizadas) no tempo real da voz.

uso: python scripts/voice_lines.py plano.json
plano.json:
{
  "voice_id": "...", "model_id": "eleven_multilingual_v2", "fps": 30, "duration": 10.6,
  "out_dir": "video/public/canal/exp01", "public_prefix": "canal/exp01",
  "lines": [
    {"id": "l1", "text": "I stopped counting the scars.", "start": 0.12, "max_dur": 1.6,
     "captions": [{"words": [0,1,2,3,4], "end_pad": 0.4, "stretch_first": true}],
     "split": {"counting": ["COUNT-"]}}
  ]
}
Saidas: <out_dir>/voice.wav (mix posicionado) e <out_dir>/voice.json (steps em frames absolutos).
Requer ELEVENLABS_API_KEY no ambiente e ffmpeg no PATH.
"""
import base64, json, os, re, subprocess, sys, tempfile, urllib.request

API = "https://api.elevenlabs.io/v1/text-to-speech/{voice}/with-timestamps"


def tts(text, voice, model, settings):
    body = json.dumps({"text": text, "model_id": model, "voice_settings": settings}).encode()
    req = urllib.request.Request(
        API.format(voice=voice) + "?output_format=mp3_44100_128",
        data=body,
        headers={"xi-api-key": os.environ["ELEVENLABS_API_KEY"], "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=120) as r:
        d = json.load(r)
    return base64.b64decode(d["audio_base64"]), d["alignment"]


def words_from_alignment(al):
    chars, st, en = al["characters"], al["character_start_times_seconds"], al["character_end_times_seconds"]
    words, cur = [], None
    for c, s, e in zip(chars, st, en):
        if re.match(r"[A-Za-z0-9']", c):
            if cur is None:
                cur = {"w": "", "s": s, "e": e}
            cur["w"] += c
            cur["e"] = e
        elif cur is not None:
            words.append(cur)
            cur = None
    if cur is not None:
        words.append(cur)
    return words


def build_captions(line_id, ws, ln, fps, start_index=0):
    """Passos de legenda (palavra/silaba) a partir de palavras com tempo ABSOLUTO no video.
    ln: {"captions": [{"words": [idx...], "kind", "end_pad", "end_max", "stretch_first"}], "split": {palavra: [partes]}}
    Sem "captions": uma legenda por frase (quebra em . ? !)."""
    caps_spec = ln.get("captions")
    if not caps_spec:
        caps_spec, cur = [], []
        for i, w in enumerate(ws):
            cur.append(i)
            if w.get("raw", w["w"]).rstrip().endswith((".", "?", "!", "\u3002")):
                caps_spec.append({"words": cur}); cur = []
        if cur:
            caps_spec.append({"words": cur})
    split = {k.lower(): v for k, v in ln.get("split", {}).items()}
    out = []
    for cap in caps_spec:
        idx = [i for i in cap["words"] if 0 <= i < len(ws)]
        if not idx:
            continue
        steps, shown = [], []
        for wi in idx:
            w = ws[wi]
            f0 = round(w["s"] * fps)
            parts = split.get(w["w"].lower())
            if parts:
                span = max(2, round((w["e"] - w["s"]) * fps))
                for k, part in enumerate(parts):
                    steps.append({"f": f0 + round(span * k / (len(parts) + 1)), "text": " ".join(shown + [part])})
                f0 = f0 + round(span * len(parts) / (len(parts) + 1))
            shown.append(w["w"].upper())
            steps.append({"f": max(f0, steps[-1]["f"] + 1) if steps else f0, "text": " ".join(shown)})
        if cap.get("stretch_first", True) and steps:
            first = steps[0]
            steps = [{"f": first["f"], "text": first["text"], "stretch": True}] + [
                {**st, "f": max(st["f"], first["f"] + 1)} for st in steps[1:]]
        last = ws[idx[-1]]
        end = round((last["e"] + cap.get("end_pad", 0.4)) * fps)
        if "end_max" in cap:
            end = min(end, cap["end_max"])
        out.append({"id": f"{line_id}-{start_index + len(out)}", "kind": cap.get("kind", "caption"), "steps": steps, "end": end})
    return out


def sh(*a):
    subprocess.run(a, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def dur(p):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", p],
                         capture_output=True, text=True, encoding="utf-8", errors="replace", check=True)
    return float(out.stdout.strip())


def main():
    plan = json.load(open(sys.argv[1], encoding="utf-8"))
    fps, out_dir = plan["fps"], plan["out_dir"]
    os.makedirs(out_dir, exist_ok=True)
    settings = plan.get("voice_settings", {"stability": 0.55, "similarity_boost": 0.8, "style": 0.25, "use_speaker_boost": True})
    tmp_dir = tempfile.TemporaryDirectory()  # some no fim (antes ficava um mkdtemp por recreate)
    tmp = tmp_dir.name
    placed, result = [], {"lines": [], "captions": []}
    for ln in plan["lines"]:
        audio, al = tts(ln["text"], plan["voice_id"], plan.get("model_id", "eleven_multilingual_v2"), settings)
        raw = os.path.join(tmp, ln["id"] + ".mp3")
        open(raw, "wb").write(audio)
        words = words_from_alignment(al)
        # corta silencio do inicio/fim pelo alinhamento
        t0, t1 = max(0, words[0]["s"] - 0.03), words[-1]["e"] + 0.12
        natural = t1 - t0
        tempo = max(1.0, natural / ln["max_dur"])  # so acelera; nunca estica
        if tempo > 1.35:
            print(f"[aviso] {ln['id']}: fala natural {natural:.2f}s > janela {ln['max_dur']}s (atempo {tempo:.2f} limitado a 1.35)")
            tempo = 1.35
        cut = os.path.join(tmp, ln["id"] + ".wav")
        sh("ffmpeg", "-y", "-ss", f"{t0:.3f}", "-to", f"{t1:.3f}", "-i", raw,
           "-af", f"atempo={tempo:.4f},afade=t=out:st={max(0,(natural/tempo)-0.06):.3f}:d=0.06", "-ar", "44100", "-ac", "2", cut)
        placed.append((cut, ln["start"]))
        ws = [{"w": w["w"], "s": ln["start"] + (w["s"] - t0) / tempo, "e": ln["start"] + (w["e"] - t0) / tempo} for w in words]
        result["lines"].append({"id": ln["id"], "text": ln["text"], "tempo": round(tempo, 3), "words": ws})
        result["captions"] += build_captions(ln["id"], ws, ln, fps, start_index=len(result["captions"]))
    # mix posicionado
    mix = os.path.join(out_dir, "voice.wav")
    args = ["ffmpeg", "-y"]
    for p, _ in placed:
        args += ["-i", p]
    fc = "".join(f"[{i}]adelay={int(s*1000)}:all=1[a{i}];" for i, (_, s) in enumerate(placed))
    fc += "".join(f"[a{i}]" for i in range(len(placed))) + f"amix=inputs={len(placed)}:normalize=0,apad=whole_dur={plan['duration']},loudnorm=I=-14:TP=-1.5:LRA=7"
    sh(*args, "-filter_complex", fc, "-ar", "44100", mix)
    json.dump(result, open(os.path.join(out_dir, "voice.json"), "w", encoding="utf-8"), indent=1)
    for l in result["lines"]:
        print(l["id"], f"tempo={l['tempo']}", " ".join(f"{w['w']}@{w['s']:.2f}" for w in l["words"]))


if __name__ == "__main__":
    main()
