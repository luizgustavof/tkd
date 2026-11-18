"""
Falas reais do footage: transcricao com idioma por frase e tempo por palavra (faster-whisper, CPU).

uso:
  python scripts/speech.py file  MIDIA.mp4 [--lang auto|en|ja|pt] [--model small]   -> JSON no stdout
  python scripts/speech.py bank  channels/<canal>/knowledge/clips.json [--force]    -> grava clip["speech"]
  python scripts/speech.py segments MIDIA.mp4 OUT.json [--min 1.2 --max 9]          -> trechos de fala para recorte
  (todos aceitam --hotwords "Toji Fushiguro, Zen'in": nomes que o whisper deve reconhecer)

Cada fala: {"start","end","text","lang","prob","words":[{"w","s","e"}]}; tempos em segundos no arquivo.
Idioma detectado POR FALA (anime alterna dublagem inglesa, japones original e dublagem pt-BR).
Fala "limpa" = prob media das palavras >= 0.5 e sem ser so musica (whisper alucina letra de musica).
"""
import argparse, json, os, sys

_MODEL = {}



def whisper_threads():
    """Metade das threads logicas (~nucleos fisicos). Medido em 17/11/2026 (16 threads, 38 s de fala):
    16 threads = 17.6 s sozinho / 44 s com 2 juntos; 8 threads = 14.1 s / 27.5 s. TKD_WHISPER_THREADS sobrescreve."""
    env = os.environ.get("TKD_WHISPER_THREADS", "")
    if env.isdigit() and int(env) > 0:
        return int(env)
    return max(4, (os.cpu_count() or 8) // 2)

def model(name):
    if name not in _MODEL:
        from faster_whisper import WhisperModel
        _MODEL[name] = WhisperModel(name, device="cpu", compute_type="int8", cpu_threads=whisper_threads())
    return _MODEL[name]


def load_audio(path):
    """16 kHz mono float32 via ffmpeg (o PyAV do venv nao casa com o faster-whisper)."""
    import subprocess, numpy as np
    raw = subprocess.run([os.environ.get("TKD_FFMPEG", "ffmpeg"), "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"],
                         capture_output=True).stdout
    return np.frombuffer(raw, dtype=np.float32)


_AUDIO = {}


def audio(path):
    if path not in _AUDIO:
        _AUDIO.clear()
        _AUDIO[path] = load_audio(path)
    return _AUDIO[path]


def transcribe(path, lang="auto", name="small", hotwords=None):
    m = model(name)
    x = audio(path)
    if len(x) < 8000:
        return {"language": "", "duration": round(len(x) / 16000, 2), "lines": []}
    kw = dict(word_timestamps=True, vad_filter=True, vad_parameters={"min_silence_duration_ms": 350},
              condition_on_previous_text=False, beam_size=5,
              # so temperatura 0: o padrao do faster-whisper cai para 0.2..1.0 (amostragem aleatoria) quando um
              # trecho falha nos limiares, e o mesmo audio dava 8/2/1/4 segmentos entre execucoes (17/11)
              temperature=0.0)
    if lang != "auto":
        kw["language"] = lang
    # nomes proprios da serie (personagens, clas, tecnicas): sem isso "Zen'in" saia "Zen and Clan"/"Zenon"
    if hotwords:
        kw["hotwords"] = hotwords
    # a deteccao de idioma sai no `info` da propria transcricao (antes uma chamada extra so para ler as
    # probabilidades repetia VAD + deteccao)
    segs, info = m.transcribe(x, **kw)
    langs = None
    if lang == "auto":
        probs = dict(info.all_language_probs or [])
        langs = {k: probs.get(k, 0) for k in ("en", "ja", "pt")}
    out = []
    for s in segs:
        words = [{"w": w.word.strip(), "s": round(w.start, 3), "e": round(w.end, 3), "p": round(w.probability, 3)}
                 for w in (s.words or []) if w.word.strip()]
        if not words:
            continue
        lg = info.language if lang == "auto" else lang
        for part in split_on_pauses(words):
            prob = sum(w["p"] for w in part) / len(part)
            text = s.text.strip() if len(part) == len(words) else join_words(part, lg)
            out.append({"start": round(part[0]["s"], 3), "end": round(part[-1]["e"], 3), "text": text,
                        "lang": lg, "prob": round(prob, 3), "no_speech": round(s.no_speech_prob, 3), "words": part})
    # idioma por fala: re-detecta frases longas quando o arquivo mistura idiomas
    if lang == "auto" and langs and len(out) > 1 and sorted(langs.values())[-2] > 0.15:
        for ln in out:
            if ln["end"] - ln["start"] >= 1.0:
                ln["lang"] = detect_span(path, ln["start"], ln["end"], name) or ln["lang"]
    return {"language": info.language, "duration": round(info.duration, 2), "lines": out}


PAUSE_SPLIT = 0.7  # s de silencio entre palavras que separa duas falas


def split_on_pauses(words):
    """Um segmento do whisper pode atravessar silencio longo (Toji 18/11: "ねぇよ" + 6.4 s calado + "2、3年もしたら"
    numa linha so; a traducao espalhada no tempo da linha aparecia com ele de boca fechada). Pausa > PAUSE_SPLIT
    vira fala separada: cada uma tem sua traducao, seu tempo e seu recorte."""
    parts = [[words[0]]]
    for w in words[1:]:
        if w["s"] - parts[-1][-1]["e"] > PAUSE_SPLIT:
            parts.append([w])
        else:
            parts[-1].append(w)
    return parts


def join_words(words, lang):
    sep = "" if lang in ("ja", "zh") else " "
    return sep.join(w["w"] for w in words).strip()


def detect_span(path, a, b, name):
    import numpy as np
    x = audio(path)[int(a * 16000):int(b * 16000)]
    if len(x) < 8000:
        return None
    try:
        lang, p, allp = model(name).detect_language(audio=np.asarray(x, dtype=np.float32))
    except Exception:
        return None
    allp = dict(allp)
    best = max(("en", "ja", "pt"), key=lambda k: allp.get(k, 0))
    return best if allp.get(best, 0) > 0.3 else lang


def clean(ln):
    return ln["prob"] >= 0.5 and ln["no_speech"] < 0.6 and len(ln["words"]) >= 1


def cmd_file(a):
    r = transcribe(a.path, a.lang, a.model, a.hotwords)
    json.dump(r, sys.stdout, ensure_ascii=False, indent=1)


def cmd_bank(a):
    bank = json.load(open(a.path, encoding="utf-8"))
    pub = os.path.join("video", "public")
    n = 0
    for c in bank:
        if "speech" in c and not a.force:
            continue
        p = os.path.join(pub, c["file"])
        if not os.path.exists(p):
            continue
        try:
            r = transcribe(p, a.lang, a.model, a.hotwords)
            c["speech"] = [ln for ln in r["lines"] if clean(ln)]
        except Exception as e:  # clipe sem audio
            print(f"  {c['file']}: {e}", file=sys.stderr)
            c["speech"] = []
        n += 1
        print(f"{c['file']}: " + " | ".join(f"[{l['lang']}] {l['text']}" for l in c["speech"]) or "(sem fala)")
        json.dump(bank, open(a.path, "w", encoding="utf-8"), indent=2, ensure_ascii=False)
    print(f"{n} clipes transcritos")


def cmd_segments(a):
    """Trechos para recortar do video-fonte: frases inteiras (agrupa frases coladas), nunca cortando no meio."""
    r = transcribe(a.path, a.lang, a.model, a.hotwords)
    lines = [l for l in r["lines"] if clean(l)]
    segs, cur = [], None
    for l in lines:
        if cur and l["start"] - cur["end"] < 0.6 and l["end"] - cur["start"] <= a.max:
            cur["end"] = l["end"]
            cur["lines"].append(l)
        else:
            if cur:
                segs.append(cur)
            cur = {"start": l["start"], "end": l["end"], "lines": [l]}
    if cur:
        segs.append(cur)
    segs = [s for s in segs if a.min <= s["end"] - s["start"] <= a.max]
    json.dump({"language": r["language"], "segments": segs}, open(a.out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"{len(segs)} trechos de fala -> {a.out}")


def main():
    ap = argparse.ArgumentParser()
    sp = ap.add_subparsers(dest="cmd", required=True)
    for name, fn in (("file", cmd_file), ("bank", cmd_bank), ("segments", cmd_segments)):
        p = sp.add_parser(name)
        p.add_argument("path")
        if name == "segments":
            p.add_argument("out")
            p.add_argument("--min", type=float, default=1.2)
            p.add_argument("--max", type=float, default=9.0)
        if name == "bank":
            p.add_argument("--force", action="store_true")
        p.add_argument("--lang", default="auto")
        p.add_argument("--model", default="small")
        p.add_argument("--hotwords", default="", help="nomes proprios separados por espaco/virgula")
        p.set_defaults(fn=fn)
    a = ap.parse_args()
    # `python -I` ignora PYTHONIOENCODING: sem isto o stdout redirecionado sai em cp1252 e fala em japones
    # derrubava o `file` (e o passo de transcricao do avaliador)
    sys.stdout.reconfigure(encoding="utf-8")
    a.fn(a)


if __name__ == "__main__":
    main()
