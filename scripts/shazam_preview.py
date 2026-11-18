"""
Previa oficial (~30 s, AAC) de uma musica identificada pelo Shazam, sem passar pelo YouTube.

uso: python -I scripts/shazam_preview.py KEY OUT.m4a   -> JSON no stdout {ok, title, artist, url} | {ok: false, error}

KEY e o `key` que o scripts/ref_audio.py grava em sound.tracks (id da faixa no Shazam). A previa vem do link
que o proprio Shazam devolve (Apple Music, audio-ssl.itunes.apple.com). Reserva da trilha da referencia:
so usada quando a ref nao tem musica separavel (stems/music.wav vazio ou ausente).
Roda com o python do venv do rea-video (shazamio).
"""
import asyncio, json, os, sys, urllib.request


def find_preview(node):
    """1o link de audio da previa em qualquer lugar da resposta (hub.actions[].uri, varia entre versoes)."""
    if isinstance(node, dict):
        uri = node.get("uri") or node.get("href") or ""
        if isinstance(uri, str) and uri.startswith("https://") and (".m4a" in uri or "audio-ssl" in uri):
            return uri
        for v in node.values():
            r = find_preview(v)
            if r:
                return r
    elif isinstance(node, list):
        for v in node:
            r = find_preview(v)
            if r:
                return r
    return None


async def about(key):
    from shazamio import Shazam
    return await Shazam().track_about(track_id=int(key))


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    key, out = sys.argv[1], sys.argv[2]
    sys.stdout.reconfigure(encoding="utf-8")
    try:
        t = asyncio.run(about(key))
    except Exception as e:
        print(json.dumps({"ok": False, "error": f"shazam: {type(e).__name__}: {e}"}))
        return
    url = find_preview(t.get("hub") or t)
    if not url:
        print(json.dumps({"ok": False, "error": "faixa sem previa no Shazam"}))
        return
    tmp = out + ".part"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(req, timeout=30) as r, open(tmp, "wb") as f:
            f.write(r.read())
        os.replace(tmp, out)
    except Exception as e:
        print(json.dumps({"ok": False, "error": f"download da previa: {e}"}))
        return
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)
    print(json.dumps({"ok": True, "title": t.get("title", ""), "artist": t.get("subtitle", ""), "url": url},
                     ensure_ascii=False))


if __name__ == "__main__":
    main()
