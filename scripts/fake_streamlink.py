"""streamlink falso para os experimentos de caos da live (docs/CHAOS.md, secao F).

O live.py chama `$TKD_STREAMLINK --stdout ... <url> <qualidade>` e le MPEG-TS do stdout; o vigia do tkd chama
`--json <url>` para saber se a live esta no ar. Este script ignora a URL e transmite um mp4 local em tempo real
(ffmpeg -re, em loop, so CPU), com as falhas pedidas no ambiente:

  TKD_FAKE_SRC          mp4 de origem (obrigatorio para transmitir)
  TKD_FAKE_STATE        arquivo com "on"/"off": live no ar ou fora (a bancada troca no meio). Sem ele = sempre no ar
  TKD_FAKE_DIE_AFTER    segundos ate a "live cair" (o processo sai com erro, como o streamlink quando a live acaba)
  TKD_FAKE_GARBAGE_AT   segundos ate mandar ~256 KB de lixo no meio do TS (segmento corrompido)

Se o processo fica congelado (PC dormindo) por mais de 20 s, a "conexao" morreu: sai com erro, como o streamlink
real quando a playlist HLS some durante o sono. Nunca acessa a rede.
"""
import json
import os
import subprocess
import sys
import time


def online():
    p = os.environ.get("TKD_FAKE_STATE")
    if not p:
        return True
    try:
        return open(p, encoding="utf-8").read().strip() != "off"
    except OSError:
        return False


def main():
    if "--json" in sys.argv:
        if not online():
            print(json.dumps({"error": "No playable streams found on this URL"}))
            return 1
        print(json.dumps({"streams": {"best": {"type": "hls"}}, "metadata": {"title": "caos", "category": None}}))
        return 0
    if not online():
        print("fake_streamlink: error: No playable streams found on this URL", file=sys.stderr)
        return 1
    src = os.environ.get("TKD_FAKE_SRC")
    if not src or not os.path.exists(src):
        print("fake_streamlink: TKD_FAKE_SRC ausente", file=sys.stderr)
        return 1
    die = float(os.environ.get("TKD_FAKE_DIE_AFTER") or 0)
    junk = float(os.environ.get("TKD_FAKE_GARBAGE_AT") or 0)
    ff = os.environ.get("TKD_FFMPEG") or "ffmpeg"
    p = subprocess.Popen(
        [ff, "-hide_banner", "-v", "error", "-re", "-stream_loop", "-1", "-i", src,
         "-c:v", "libx264", "-preset", "ultrafast", "-g", "60", "-c:a", "aac", "-f", "mpegts", "pipe:1"],
        stdout=subprocess.PIPE)
    out = sys.stdout.buffer
    t0 = last = checked = time.time()
    junk_sent = False
    try:
        while True:
            chunk = p.stdout.read(64 * 1024)
            if not chunk:
                return 1
            now = time.time()
            if now - last > 20:
                print(f"fake_streamlink: {now - last:.0f}s sem rodar (PC dormiu?): conexao perdida", file=sys.stderr,
                      flush=True)
                return 1
            last = now
            out.write(chunk)
            el = now - t0
            if now - checked >= 1:
                checked = now
                if not online():
                    print(f"fake_streamlink: live saiu do ar aos {el:.0f}s", file=sys.stderr, flush=True)
                    return 1
            if junk and not junk_sent and el >= junk:
                out.write(os.urandom(256 * 1024))
                junk_sent = True
                print(f"fake_streamlink: lixo injetado aos {el:.0f}s", file=sys.stderr, flush=True)
            if die and el >= die:
                print(f"fake_streamlink: live caiu aos {el:.0f}s", file=sys.stderr, flush=True)
                return 1
    except (BrokenPipeError, OSError):
        return 0
    finally:
        p.kill()
        p.wait()


if __name__ == "__main__":
    sys.exit(main())
