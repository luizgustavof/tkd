"""
O audio de uma referencia: o que toca, quanto e fala e onde a musica explode.

uso: python -I scripts/ref_audio.py REF.mp4 [--no-shazam] [--whisper small]   -> JSON no stdout

Saida:
  duration      segundos
  speech        {seconds, ratio, lines:[{start,end,text}]}  fala detectada (VAD + whisper)
  music         {bpm, beats, drop_s, energy_rise_db}         ritmo e drop (librosa, CPU)
  tracks        [{title, artist, key, windows:[...], confidence}]  musica identificada (Shazam)

Shazam: so a impressao digital do audio sai do PC (nunca o audio). 3 janelas de 10 s
(comeco, meio, fim); a mesma faixa em 2+ janelas = "alta", em 1 = "baixa" (pode ser erro
ou um edit com duas musicas). Versoes aceleradas/slowed as vezes nao sao reconhecidas.
Roda com o python do venv do rea-video (librosa, faster-whisper, shazamio).
"""
import argparse, asyncio, json, os, subprocess, sys, tempfile

import numpy as np

FFMPEG = os.environ.get("TKD_FFMPEG", "ffmpeg")
SR = 22050



def whisper_threads():
    """Metade das threads logicas (~nucleos fisicos). Medido em 17/11/2026 (16 threads, 38 s de fala):
    16 threads = 17.6 s sozinho / 44 s com 2 juntos; 8 threads = 14.1 s / 27.5 s. TKD_WHISPER_THREADS sobrescreve."""
    env = os.environ.get("TKD_WHISPER_THREADS", "")
    if env.isdigit() and int(env) > 0:
        return int(env)
    return max(4, (os.cpu_count() or 8) // 2)

def load(path, sr):
    raw = subprocess.run([FFMPEG, "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(sr), "-f", "f32le", "-"],
                         capture_output=True).stdout
    return np.frombuffer(raw, dtype=np.float32)


def music_features(y):
    """BPM, batidas e drop (maior subida de energia sustentada)."""
    import librosa
    if len(y) < SR:
        return {"bpm": 0, "beats": 0, "drop_s": None, "energy_rise_db": 0}
    tempo, beats = librosa.beat.beat_track(y=y, sr=SR, units="time")
    bpm = float(np.atleast_1d(tempo)[0])
    hop = 512
    rms = librosa.feature.rms(y=y, hop_length=hop)[0]
    db = librosa.amplitude_to_db(rms + 1e-9)
    fps = SR / hop
    w = int(1.5 * fps)  # janela de 1,5 s antes e depois
    best, at = 0.0, None
    for i in range(w, len(db) - w):
        rise = db[i:i + w].mean() - db[i - w:i].mean()
        if rise > best:
            best, at = rise, i
    # mixagens de edit sao muito comprimidas (-6 LUFS): 1,5 dB ja e uma subida audivel
    drop = round(at / fps, 2) if at is not None and best >= 1.5 else None
    return {"bpm": round(bpm, 1), "beats": int(len(beats)), "drop_s": drop, "energy_rise_db": round(float(best), 1)}


def speech(path, model_name):
    """Fala de verdade: VAD do whisper + texto com confianca minima (canto/grito costuma sair com logprob baixo)."""
    from faster_whisper import WhisperModel
    x = load(path, 16000)
    dur = len(x) / 16000
    if len(x) < 8000:
        return {"seconds": 0, "ratio": 0, "lines": []}
    m = WhisperModel(model_name, device="cpu", compute_type="int8", cpu_threads=whisper_threads())
    segs, _ = m.transcribe(x, vad_filter=True, vad_parameters={"min_silence_duration_ms": 350},
                           condition_on_previous_text=False, beam_size=3, temperature=0.0)  # sem fallback aleatorio
    lines = []
    for s in segs:
        t = s.text.strip()
        if t and s.no_speech_prob < 0.6 and s.avg_logprob > -1.0:
            lines.append({"start": round(s.start, 2), "end": round(s.end, 2), "text": t})
    sec = sum(l["end"] - l["start"] for l in lines)
    return {"seconds": round(sec, 1), "ratio": round(sec / dur, 2) if dur else 0, "lines": lines}


async def identify(path, dur):
    """Shazam em 3 janelas; junta por faixa."""
    from shazamio import Shazam
    sh = Shazam()
    wins = sorted({0.0, max(0.0, dur * 0.45), max(0.0, dur - 10.0)}) if dur > 12 else [0.0]
    hits = {}
    with tempfile.TemporaryDirectory() as tmp:
        for i, start in enumerate(wins):
            seg = os.path.join(tmp, f"w{i}.wav")
            subprocess.run([FFMPEG, "-v", "error", "-y", "-ss", f"{start:.2f}", "-t", "10", "-i", path,
                            "-vn", "-ac", "1", "-ar", "44100", seg], check=False)
            if not os.path.exists(seg):
                continue
            try:
                r = await sh.recognize(seg)
            except Exception as e:  # limite/instabilidade da API: segue sem essa janela
                print(f"shazam falhou na janela {start:.0f}s: {e}", file=sys.stderr)
                continue
            t = r.get("track") or {}
            if t.get("key"):
                h = hits.setdefault(t["key"], {"title": t.get("title", ""), "artist": t.get("subtitle", ""),
                                               "key": t["key"], "windows": []})
                h["windows"].append(round(start, 1))
            await asyncio.sleep(1.5)  # gentil com a API
    out = sorted(hits.values(), key=lambda h: -len(h["windows"]))
    for h in out:
        h["confidence"] = "alta" if len(h["windows"]) >= 2 or len(wins) == 1 else "baixa"
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--no-shazam", action="store_true")
    ap.add_argument("--whisper", default="small")
    a = ap.parse_args()
    y = load(a.file, SR)
    dur = len(y) / SR
    out = {"duration": round(dur, 2), "music": music_features(y), "speech": speech(a.file, a.whisper), "tracks": []}
    if not a.no_shazam:
        try:
            out["tracks"] = asyncio.run(identify(a.file, dur))
        except Exception as e:
            print(f"shazam indisponivel: {e}", file=sys.stderr)
    sys.stdout.reconfigure(encoding="utf-8")
    print(json.dumps(out, ensure_ascii=False))


if __name__ == "__main__":
    main()
