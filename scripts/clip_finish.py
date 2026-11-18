"""Acabamento de um corte de live guardado como bruto (live.py --lazy): so roda nos cortes que vao ao ar.

  --speech    whisper -> fala com tempos no json (+ .srt)        (finalistas: o Claude le a fala)
  --vertical  9:16 com fundo desfocado e legenda queimada          (so o escolhido)
              com --ss/--to/--out: so o trecho, direto no arquivo final (um encode a menos)

Uso: python -I scripts/clip_finish.py out/live/.../clips/clip_03.json --speech [--vertical] [--captions]
Imprime o json atualizado.
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.stdout.reconfigure(encoding="utf-8")
import live as L  # noqa: E402


def save(info, path):
    tmp = path + ".tmp"
    json.dump(info, open(tmp, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    os.replace(tmp, path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("json")
    ap.add_argument("--speech", action="store_true")
    ap.add_argument("--vertical", action="store_true")
    ap.add_argument("--captions", action="store_true")
    ap.add_argument("--lang", default="auto")
    ap.add_argument("--ss", type=float)
    ap.add_argument("--to", type=float)
    ap.add_argument("--out", help="com --ss/--to: grava o 9:16 recortado aqui (nao vira o vertical do clipe)")
    a = ap.parse_args()

    info = json.load(open(a.json, encoding="utf-8"))
    base = a.json[:-5]
    folder = os.path.dirname(os.path.abspath(a.json))
    raw = os.path.join(folder, info["files"]["raw"])
    files = info.setdefault("files", {})

    # speech_done: corte sem fala tem speech [] e passava de novo pelo whisper a cada vez que era finalista
    if a.speech and not info.get("speech") and not info.get("speech_done"):
        speech_lines, words = L.transcribe_clip(raw, base, a.lang)
        info.update({"speech": speech_lines, "transcript": " ".join(ln["text"] for ln in speech_lines), "speech_done": True})
        files["srt"] = os.path.basename(base + ".srt") if words else None
    if a.vertical and a.out:
        L.CAPTIONS = a.captions
        words = [w for ln in info.get("speech") or [] for w in ln.get("words", [])]
        L.vertical(raw, base, words, a.ss, a.to, os.path.abspath(a.out))
        print(json.dumps({"out": a.out, "words": len(words)}))
        save(info, a.json)
        return
    if a.vertical and not (files.get("vertical") and os.path.exists(os.path.join(folder, files["vertical"]))):
        L.CAPTIONS = a.captions
        words = [w for ln in info.get("speech") or [] for w in ln.get("words", [])]
        files["vertical"] = os.path.basename(L.vertical(raw, base, words))
        info["captions_burned"] = bool(a.captions and words)
    info["finished"] = bool(files.get("vertical"))

    save(info, a.json)
    print(json.dumps({"vertical": files.get("vertical"), "words": sum(len(ln.get("words", [])) for ln in info.get("speech") or [])}))


if __name__ == "__main__":
    main()
