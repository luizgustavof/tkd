"""Avaliador de exemplo para a ponte `[evaluator]` (protocolo em docs/EVALUATOR.md).

Nao e um modelo: mede tres defeitos com o ffmpeg (abertura congelada, tela preta, audio estourado) e
da uma nota simples. Serve para testar a ponte e de ponto de partida para o seu avaliador.

    python scripts/example_evaluator.py VIDEO.mp4 [--caption "legenda"]

Saida: logs no stderr, e o JSON na ultima linha do stdout.
"""
import argparse
import json
import re
import subprocess
import sys
import time


def ffmpeg_log(video: str, *filters: str) -> str:
    cmd = ["ffmpeg", "-hide_banner", "-nostats", "-i", video, *filters, "-f", "null", "-"]
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace").stderr


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("video")
    ap.add_argument("--caption", default="")
    a = ap.parse_args()
    t0 = time.time()
    problems = []

    # abertura parada: o primeiro segundo e o gancho
    log = ffmpeg_log(a.video, "-t", "4", "-vf", "freezedetect=n=0.003:d=1", "-an")
    starts = [float(x) for x in re.findall(r"freeze_start: ([\d.]+)", log)]
    if starts and starts[0] < 0.5:
        end = re.findall(r"freeze_end: ([\d.]+)", log)
        problems.append({"t": [starts[0], float(end[0]) if end else 4.0], "type": "frozen_open", "severity": "high",
                         "what": "abre com o quadro parado", "fix": "comecar direto na acao"})

    # tela preta longa
    log = ffmpeg_log(a.video, "-vf", "blackdetect=d=0.5", "-an")
    for s, e in re.findall(r"black_start:([\d.]+) black_end:([\d.]+)", log):
        problems.append({"t": [float(s), float(e)], "type": "black_screen", "severity": "medium",
                         "what": "tela preta", "fix": "cortar o trecho preto"})

    # pico de audio no teto
    log = ffmpeg_log(a.video, "-af", "volumedetect", "-vn")
    peak = re.findall(r"max_volume: (-?[\d.]+) dB", log)
    if peak and float(peak[0]) >= -0.1:
        problems.append({"type": "clipped_audio", "severity": "medium", "what": "audio estourado (pico em 0 dB)",
                         "fix": "baixar o ganho ou usar um limitador"})

    penalty = {"high": 2.5, "medium": 1.0, "low": 0.3}
    overall = max(0.0, min(10.0, 8.0 - sum(penalty[p["severity"]] for p in problems)))
    print(f"exemplo: {len(problems)} problema(s)", file=sys.stderr)
    print(json.dumps({
        "model": "example",
        "overall": round(overall, 1),
        "summary": "medido pelo avaliador de exemplo (ffmpeg)",
        "problems": problems,
        "strengths": [],
        "seconds": round(time.time() - t0, 1),
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
