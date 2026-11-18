"""
Clipper de live: assiste uma live (Twitch, Kick, YouTube), le o chat sem login e corta os momentos fortes.

uso:
  python scripts/live.py URL OUTDIR [--minutes 0] [--seg 30] [--threshold 1.2] [--min 15] [--max 45]
                                    [--lang auto] [--chat-offset 2] [--quality 720p60,720p,best] [--no-chat]

Fluxo:
  captura  streamlink -> ffmpeg -f segment: arquivos .ts de --seg segundos (buffer rotativo em OUTDIR/buf)
  chat     Twitch: IRC anonimo (justinfan) | Kick: API publica de mensagens (polling 2 s) | YouTube: API publica do live_chat (>= 8 s)
           grava OUTDIR/chat.jsonl {"w": hora unix, "m": mensagem}  (sem nome de usuario)
  analise  a cada segmento novo: janela dos ultimos ~4 min -> moments.py perfil "live" (chat, audio, movimento)
           pico novo acima de --threshold, com desfecho ja gravado -> corte
  corte    OUTDIR/clips/clip_NN.mp4 (original), clip_NN_9x16.mp4 (fundo desfocado), .srt (whisper), .json

Sem GPU (decodificacao so na CPU). FFmpeg: TKD_FFMPEG; streamlink: TKD_STREAMLINK.
"""
import argparse, collections, json, os, re, shutil, signal, socket, subprocess, sys, threading, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import moments as M  # noqa: E402
import yt_pace  # noqa: E402
from web import http_session  # noqa: E402  TLS de Chrome (curl_cffi/Scrapling); cai no requests sem ele

FF = os.environ.get("TKD_FFMPEG", "ffmpeg")
SL = os.environ.get("TKD_STREAMLINK", "streamlink")
# streamer gritando estoura o audio da live (+2.7 dBFS medido): sem limitador o pico ia direto pro 9:16
# (que copia o audio) e a barreira tecnica jogava o corte fora. -2 dB de teto deixa folga pro AAC.
LIMITER = "aresample=async=1,alimiter=limit=0.79:level=false"
STOP = threading.Event()


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def platform_of(url):
    u = url.lower()
    if "twitch.tv/" in u:
        return "twitch", re.search(r"twitch\.tv/([\w]+)", u).group(1)
    if "kick.com/" in u:
        return "kick", re.search(r"kick\.com/([\w-]+)", u).group(1)
    if "youtube.com/" in u or "youtu.be/" in u:
        m = re.search(r"(?:v=|youtu\.be/|/live/)([\w-]{11})", url)
        return "youtube", (m.group(1) if m else re.sub(r"\W+", "_", url.split("youtube.com/")[-1])[:30])
    return "outro", re.sub(r"\W+", "_", url)[-30:]


# ---------------- chat ----------------

class Chat:
    """Coleta mensagens em thread; guarda (hora unix, msg) em memoria e em chat.jsonl.
    Na memoria fica so o recente (a janela de analise anda para frente); o historico inteiro esta no
    chat.jsonl. Antes uma live de 8 h guardava ~800 mil mensagens e varria todas a cada segmento."""

    KEEP_S = 900   # bem mais que a janela (~4 min) + folga do atraso do chat

    def __init__(self, path):
        self.msgs = collections.deque()
        self.total = 0
        self.lock = threading.Lock()
        self.f = open(path, "a", encoding="utf-8")
        self.error_msg = None
        self.last_msg_at = None   # hora da ultima mensagem

    MUTE_S = 60

    def is_down(self):
        """Chat fora do ar (com erro e sem mensagem ha MUDO_S): o sinal dele vira zero na janela e, com peso 0.40 no
        perfil live, puxava a nota de todo pico para baixo (live com chat caido quase nao cortava, sem chat cortava).
        Caido = analisa como --no-chat. Chat so quieto (sem erro) continua valendo."""
        return bool(self.error_msg) and (self.last_msg_at is None or time.time() - self.last_msg_at > self.MUTE_S)

    def add(self, msg, w=None):
        msg = (msg or "").strip()
        if not msg:
            return
        w = time.time() if w is None else w
        with self.lock:
            self.msgs.append((w, msg))
            self.total += 1
            self.last_msg_at = time.time()
            while self.msgs and self.msgs[0][0] < w - self.KEEP_S:
                self.msgs.popleft()
        self.f.write(json.dumps({"w": round(w, 2), "m": msg}, ensure_ascii=False) + "\n")
        self.f.flush()

    def entre(self, a, b):
        with self.lock:
            return [(w, m) for w, m in self.msgs if a <= w < b]


def chat_twitch(canal, chat):
    while not STOP.is_set():
        try:
            host, port = TWITCH_IRC.rsplit(":", 1)
            s = socket.create_connection((host, int(port)), timeout=30)
            s.sendall(f"NICK justinfan{int(time.time()) % 100000}\r\nJOIN #{canal}\r\n".encode())
            buf = b""
            log(f"chat twitch: conectado em #{canal} (anonimo)")
            chat.error_msg = None
            while not STOP.is_set():
                try:
                    d = s.recv(65536)
                except socket.timeout:
                    s.sendall(b"PING :tmi.twitch.tv\r\n")
                    continue
                if not d:
                    break
                buf += d
                *lines_, buf = buf.split(b"\r\n")
                for ln in lines_:
                    t = ln.decode("utf-8", "replace")
                    if t.startswith("PING"):
                        s.sendall(b"PONG :tmi.twitch.tv\r\n")
                    elif " PRIVMSG " in t:
                        chat.add(t.split(" :", 1)[1] if " :" in t else "")
            s.close()
        except Exception as e:
            chat.error_msg = f"twitch: {e}"
        if not STOP.is_set():
            time.sleep(5)


KICK_EMOTE = re.compile(r"\[emote:\d+:([^\]]+)\]")
KICK_POLL = 2.0   # s entre consultas (inicio); o endpoint devolve as 25 ultimas mensagens
KICK_POLL_MIN, KICK_POLL_MAX = 0.7, 5.0


def kick_info(slug, sess=None):
    # o Kick fica atras do Cloudflare: o TLS do requests (Python) toma 403/429 antes da hora
    sess = sess or http_session({"Accept": "application/json"})
    r = sess.get(f"{KICK_BASE}/api/v2/channels/{slug}", timeout=20)
    r.raise_for_status()
    return r.json()


def chat_kick(slug, chat):
    """Kick saiu do Pusher publico (chave antiga recusada em 11/2026). Sem login, o que sobra e consultar
    /api/v2/channels/<id>/messages (25 ultimas, com id e created_at do servidor) a cada KICK_POLL s.
    Intervalo adaptativo 0.7-5 s: ate ~30 msg/s sem perda; acima disso parte se perde (a velocidade do chat satura, mas o pico ainda aparece)."""
    from datetime import datetime
    sess = http_session({"Accept": "application/json"})
    try:
        cid = kick_info(slug, sess)["id"]
    except Exception as e:
        chat.error_msg = f"kick: canal nao encontrado ({e})"
        log("chat kick: canal nao encontrado:", e)
        return
    url = f"{KICK_BASE}/api/v2/channels/{cid}/messages"
    from collections import OrderedDict
    seen_n, failures, n429 = OrderedDict(), 0, 0
    poll = KICK_POLL
    log(f"chat kick: lendo canal {cid} (publico, {KICK_POLL_MIN}-{KICK_POLL_MAX:.0f}s conforme o volume)")
    while not STOP.is_set():
        try:
            r = sess.get(url, timeout=10)
            if r.status_code == 429:
                n429 += 1
                poll = KICK_POLL_MAX
                # sem isso o chat ficava mudo por ate 5 min sem erro: a nota seguia contando o chat parado
                chat.error_msg = f"kick: 429 (limite do IP), esperando {min(300, 30 * n429)}s"
                STOP.wait(min(300, 30 * n429))
                continue
            r.raise_for_status()
            msgs = r.json().get("data", {}).get("messages", [])
            new_msgs = []
            for m in msgs:
                if m.get("id") in seen_n:
                    continue
                seen_n[m.get("id")] = 1
                try:
                    w = datetime.fromisoformat(m["created_at"].replace("Z", "+00:00")).timestamp()
                except Exception:
                    w = time.time()
                new_msgs.append((w, KICK_EMOTE.sub(r"  ", m.get("content") or "")))
            for w, msg in sorted(new_msgs):
                chat.add(msg, w)
            while len(seen_n) > 2000:
                seen_n.popitem(last=False)          # descarta os ids mais antigos
            # resposta quase toda nova = chat mais rapido que a consulta: encurta; quase nada novo: alonga
            fill_ratio = len(new_msgs) / max(1, len(msgs))
            if len(msgs) >= 20 and fill_ratio > 0.8:
                poll = max(KICK_POLL_MIN, poll * 0.6)
                if poll == KICK_POLL_MIN:
                    chat.lost = getattr(chat, "perdidas", 0) + 1   # ainda saturado no minimo
            elif fill_ratio < 0.3:
                poll = min(KICK_POLL_MAX, poll * 1.25)
            failures, n429 = 0, 0
            chat.error_msg = None
        except Exception as e:
            failures += 1
            chat.error_msg = f"kick: {e}"
            STOP.wait(min(30, 2 * failures))
        STOP.wait(poll)


# experimentos de caos (docs/CHAOS.md, F3): chat falso local no lugar do da Twitch/YouTube/Kick
TWITCH_IRC = os.environ.get("TKD_TWITCH_IRC", "irc.chat.twitch.tv:6667")
YT_BASE = os.environ.get("TKD_YOUTUBE_BASE", "https://www.youtube.com").rstrip("/")
KICK_BASE = os.environ.get("TKD_KICK_BASE", "https://kick.com").rstrip("/")
YT_POLL_MIN = 8.0   # s; o YouTube costuma pedir 10 s (timeoutMs). ~360-450 requisicoes/h


def youtube_id(url, sess):
    m = re.search(r"(?:v=|youtu\.be/|/live/)([\w-]{11})", url)
    if m:
        return m.group(1)
    # @canal/live: a pagina aponta o video ao vivo no link canonico
    h = sess.get(url, timeout=20).text
    m = re.search(r'<link rel="canonical" href="https://www\.youtube\.com/watch\?v=([\w-]{11})"', h) \
        or re.search(r'"videoId":"([\w-]{11})"', h)
    return m.group(1) if m else None


def chat_youtube(url, chat):
    """Chat ao vivo do YouTube sem login nem biblioteca (chat-downloader 0.2.8 quebrou em 11/2026):
    pagina publica live_chat -> token -> youtubei/v1/live_chat/get_live_chat em loop.
    Ritmo = o que o YouTube pede (timeoutMs), nunca menos que YT_POLL_MIN. 429 = para de vez."""
    s = http_session({"Accept-Language": "en-US,en;q=0.9"})
    s.cookies.set("CONSENT", "YES+1", domain=".youtube.com")   # pula a tela de consentimento (nao e login)
    try:
        yt_pace.reserve(2)                       # pagina do canal + pagina do chat
        vid = youtube_id(url, s)
        h = s.get(f"{YT_BASE}/live_chat?v={vid}&is_popout=1", timeout=20).text
        key = re.search(r'"INNERTUBE_API_KEY":"([^"]+)"', h).group(1)
        ver = re.search(r'"INNERTUBE_CLIENT_VERSION":"([^"]+)"', h).group(1)
        continuation = re.search(r'"continuation":"([^"]+)"', h).group(1)
    except yt_pace.Blocked as e:
        chat.error_msg = f"youtube: {e}"
        log("chat youtube:", e)
        return
    except Exception as e:
        chat.error_msg = f"youtube: chat indisponivel ({e})"
        log("chat youtube: nao consegui abrir o chat:", e)
        return
    api = f"{YT_BASE}/youtubei/v1/live_chat/get_live_chat?key={key}&prettyPrint=false"
    log(f"chat youtube: lendo {vid} (publico, a cada >= {YT_POLL_MIN:.0f}s)")
    failures = 0
    while not STOP.is_set() and continuation:
        wait_s = YT_POLL_MIN
        try:
            # cada consulta conta no orcamento por IP dividido com o tkd (yt-dlp); se apertar, o chat cede
            try:
                yt_pace.reserve(yt_pace.WEIGHT_CHAT, max_wait=0, interval=0)
            except yt_pace.Exhausted:
                chat.error_msg = "youtube: orcamento da hora no limite; chat mais lento"
                STOP.wait(60)
                continue
            r = s.post(api, json={"context": {"client": {"clientName": "WEB", "clientVersion": ver}},
                                  "continuation": continuation}, timeout=20)
            if r.status_code == 429:
                yt_pace.mark_blocked()
                chat.error_msg = "youtube: 429 (limite do IP); chat parado para nao estender o bloqueio"
                log("chat youtube:", chat.error_msg)
                return
            r.raise_for_status()
            lc = r.json().get("continuationContents", {}).get("liveChatContinuation", {})
            for a in lc.get("actions", []):
                it = a.get("addChatItemAction", {}).get("item", {}).get("liveChatTextMessageRenderer")
                if not it:
                    continue
                txt = "".join(rn.get("text") or (rn.get("emoji", {}).get("shortcuts") or [""])[0]
                              for rn in it.get("message", {}).get("runs", []))
                chat.add(txt, int(it.get("timestampUsec", 0)) / 1e6 or None)
            nxt = next(iter((lc.get("continuations") or [{}])[0].values()), {})
            continuation = nxt.get("continuation")
            wait_s = max(YT_POLL_MIN, float(nxt.get("timeoutMs", 0)) / 1000)
            failures = 0
            chat.error_msg = None
        except Exception as e:
            failures += 1
            chat.error_msg = f"youtube: {e}"
            wait_s = min(120, 15 * failures)
        STOP.wait(wait_s)
    if not continuation:
        log("chat youtube: o YouTube encerrou o chat (live acabou?)")


# ---------------- captura ----------------

def start_capture(url, buf, seg, quality, plat):
    sl_args = [SL, "--stdout", "--loglevel", "warning", "--retry-streams", "5", "--retry-max", "6",
               "--hls-live-edge", "2"]
    if plat == "twitch":
        sl_args += ["--twitch-low-latency"]
    if plat == "youtube":
        yt_pace.reserve(yt_pace.WEIGHT_DOWNLOAD)    # pagina + player; os pedacos HLS vem do googlevideo
    sl_args += [url, quality]
    # stderr em arquivo, nunca em PIPE sem leitor: em horas de live o buffer do pipe enchia, o streamlink
    # travava escrevendo log e a captura parava calada (ffmpeg vivo, sem segmento novo)
    err = open(os.path.join(os.path.dirname(buf), "capture.log"), "ab")
    sl = subprocess.Popen(sl_args, stdout=subprocess.PIPE, stderr=err)
    ff = subprocess.Popen(
        [FF, "-hide_banner", "-v", "error", "-i", "pipe:0", "-map", "0:v:0?", "-map", "0:a:0?", "-c", "copy",
         "-f", "segment", "-segment_time", str(seg), "-reset_timestamps", "1", "-segment_format", "mpegts",
         "-segment_list", os.path.join(buf, "segs.csv"), "-segment_list_type", "csv",
         os.path.join(buf, "seg_%05d.ts")],
        stdin=sl.stdout, stderr=err)
    sl.stdout.close()
    err.close()                     # os filhos tem a copia deles
    return sl, ff


def log_tail(buf, n=1):
    try:
        with open(os.path.join(os.path.dirname(buf), "capture.log"), "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - 4096))
            lines_ = f.read().decode("utf-8", "replace").strip().splitlines()
        return lines_[-n:]
    except OSError:
        return []


def read_segments(buf):
    p = os.path.join(buf, "segs.csv")
    if not os.path.exists(p):
        return []
    out = []
    for ln in open(p, encoding="utf-8").read().splitlines():
        parts = ln.split(",")
        if len(parts) >= 3:
            try:
                out.append((parts[0], float(parts[2]) - float(parts[1])))
            except ValueError:
                pass
    return out


# ---------------- corte ----------------

def concat(paths, dst):
    lst = dst + ".txt"
    with open(lst, "w", encoding="utf-8") as f:
        for p in paths:
            # absoluto: o concat resolve caminho relativo a partir da pasta da LISTA, nao do cwd
            p = os.path.abspath(p).replace("\\", "/")
            f.write("file '" + p.replace("'", "'\\''") + "'\n")
    r = subprocess.run([FF, "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", lst, "-c", "copy", dst],
                       capture_output=True)
    os.remove(lst)
    return r.returncode == 0 and os.path.exists(dst)


def write_srt(words, path, per_n=3):
    def ts(t):
        h, r = divmod(max(0.0, t), 3600)
        m, s = divmod(r, 60)
        return f"{int(h):02d}:{int(m):02d}:{int(s):02d},{int((s % 1) * 1000):03d}"
    with open(path, "w", encoding="utf-8") as f:
        for i in range(0, len(words), per_n):
            g = words[i:i + per_n]
            f.write(f"{i // per_n + 1}\n{ts(g[0]['s'])} --> {ts(g[-1]['e'])}\n{' '.join(w['w'] for w in g)}\n\n")


def cut_raw(window_ts, a, b, base):
    """Corte imediato (a janela e sobrescrita na proxima analise). Seek DEPOIS do -i: no .ts concatenado o
    seek de entrada caia no keyframe seguinte e o video saia 0.5-1.6 s menor que o audio."""
    raw = base + ".mp4"
    subprocess.run([FF, "-y", "-v", "error", "-fflags", "+genpts", "-i", window_ts, "-ss", f"{a:.2f}",
                    "-t", f"{b - a:.2f}", "-c:v", "libx264", "-preset", "veryfast", "-crf", "20",
                    "-af", LIMITER, "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart", raw],
                   check=True)
    return raw


CAPTIONS = False  # --captions liga a legenda queimada no 9:16 (desligada por padrao; o .srt sai sempre)


def word_groups(words, per_n=3, gap=0.6):
    """Palavras em blocos curtos (estilo TikTok): no maximo `por`, quebra em pausa."""
    out, g = [], []
    for w in words:
        if g and (len(g) >= per_n or w["s"] - g[-1]["e"] > gap):
            out.append(g)
            g = []
        g.append(w)
    if g:
        out.append(g)
    return out


def write_ass(words, path, y=1480):
    """Legenda ASS para o 9:16 (1080x1920): fonte grossa, contorno preto, abaixo do video (fundo desfocado)."""
    def ts(t):
        t = max(0.0, t)
        return f"{int(t // 3600)}:{int(t % 3600 // 60):02d}:{t % 60:05.2f}"
    lines_ = []
    gs = word_groups(words)
    for i, g in enumerate(gs):
        end_t = min(g[-1]["e"] + 0.25, gs[i + 1][0]["s"]) if i + 1 < len(gs) else g[-1]["e"] + 0.4
        txt = " ".join(w["w"].strip() for w in g).upper().replace("{", "(").replace("}", ")").rstrip(",.;:")
        lines_.append(f"Dialogue: 0,{ts(g[0]['s'])},{ts(end_t)},Cap,,0,0,0,,{{\\pos(540,{y})}}{txt}")
    header_lines = [
        "[Script Info]", "ScriptType: v4.00+", "PlayResX: 1080", "PlayResY: 1920", "WrapStyle: 0", "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, "
        "Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, "
        "MarginL, MarginR, MarginV, Encoding",
        "Style: Cap,Arial Black,86,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,7,3,5,60,60,0,1",
        "",
        "[Events]",
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(header_lines + lines_) + "\n")


def transcribe_clip(raw, base, lang):
    """whisper -> (falas, palavras); grava o .srt. Falha = listas vazias."""
    words, speech_lines = [], []
    try:
        import speech as SP
        tr = SP.transcribe(raw, lang)
        speech_lines = tr["lines"]
        words = [w for ln in speech_lines for w in ln["words"]]
        if words:
            write_srt(words, base + ".srt")
    except Exception as e:
        log("whisper falhou:", e)
    return speech_lines, words


def vertical(raw, base, words, ss=None, to=None, out=None):
    """9:16 com fundo desfocado (+ legenda queimada se LEGENDA). Devolve o caminho do _9x16.mp4.
    Com ss/to recorta o trecho no mesmo encode (antes: 9:16 inteiro e depois outro x264 so para recortar)
    e grava em `out`."""
    vert = out or base + "_9x16.mp4"
    tmp = vert[:-4] + ".part.mp4"                 # so vira o final quando termina (processo morto = sem lixo)
    seek = []
    if ss is not None:
        seek = ["-ss", f"{ss:.2f}", "-t", f"{to - ss:.2f}"]
        words = [dict(w, s=w["s"] - ss, e=w["e"] - ss) for w in words if w["e"] > ss and w["s"] < to]
    vf = ("split[a][b];[a]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=24:2[bg];"
          "[b]scale=1080:-2[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,fps=30")
    folder, stream_name = os.path.split(os.path.abspath(base))
    if CAPTIONS and words:
        write_ass(words, base + ".ass")
        vf += f",subtitles={stream_name}.ass"            # caminho relativo (cwd = pasta): sem escapar o C: do Windows
    r = subprocess.run([FF, "-y", "-v", "error", *seek, "-i", os.path.abspath(raw), "-filter_complex", vf,
                        "-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-c:a", "copy",
                        "-movflags", "+faststart", os.path.abspath(tmp)], cwd=folder, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0 and CAPTIONS and words:
        log("legenda queimada falhou, 9:16 sem legenda:", (r.stderr or "").strip()[-160:])
        vf = vf.rsplit(",subtitles=", 1)[0]
        r = subprocess.run([FF, "-y", "-v", "error", *seek, "-i", raw, "-filter_complex", vf, "-c:v", "libx264",
                            "-preset", "veryfast", "-crf", "21", "-c:a", "copy", "-movflags", "+faststart", tmp],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        raise RuntimeError(f"9:16 falhou: {(r.stderr or '').strip()[-200:]}")
    os.replace(tmp, vert)
    return vert


LAZY = False  # --lazy: durante a live so guarda o corte bruto; whisper e 9:16 so na hora do post


def finish_clip(raw, base, lang, chat_txt, info):
    """Normal: whisper -> 9:16 -> json. --lazy: so o json (o acabamento e feito pelo clip_finish.py
    nos poucos cortes que vao ao ar). Roda em thread de fundo para nao travar a analise."""
    info["chat"] = chat_txt[:300]
    if LAZY:
        info.update({"files": {"raw": os.path.basename(raw), "vertical": None, "srt": None},
                     "finished": False, "transcript": "", "speech": []})
    else:
        speech_lines, words = transcribe_clip(raw, base, lang)
        vert = vertical(raw, base, words)
        info.update({"files": {"raw": os.path.basename(raw), "vertical": os.path.basename(vert),
                               "srt": os.path.basename(base + ".srt") if words else None},
                     "captions_burned": bool(CAPTIONS and words), "finished": True,
                     "transcript": " ".join(ln["text"] for ln in speech_lines), "speech": speech_lines})
    json.dump(info, open(base + ".json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    log(f"  pronto {os.path.basename(base)}" + (" (bruto)" if LAZY else f": \"{info['transcript'][:70]}\""))


# ---------------- categoria da live (jogo) ----------------

CATEGORY = {"ok": True, "nome": None}


def category_matches(meta, games_list):
    """(ok, nome) pela metadata do streamlink. Reprise nunca serve; sem metadata (YouTube) = deixa passar."""
    if not meta:
        return True, None
    title_text = (meta.get("title") or "").lower()
    if title_text.startswith("rerun") or "[rerun]" in title_text or title_text.startswith("reprise"):
        return False, "reprise"
    cat = meta.get("category")
    if not games_list or not cat:
        return True, cat
    c = game_tokens(cat)
    for j in games_list:
        jj = game_tokens(j)
        if jj and any(c[i:i + len(jj)] == jj for i in range(len(c) - len(jj) + 1)):
            return True, cat
    return False, cat


ROMAN = ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"]


def game_tokens(s):
    """Mesmo que `game_tokens` do cortes.rs: "gta" -> "grand theft auto", romano -> arabico, palavra inteira.
    Antes "grand theft auto v" (GTA 5) estava dentro de "grand theft auto vi" e passava como GTA VI."""
    out = []
    for w in re.sub(r"[^a-z0-9]+", " ", s.lower()).split():
        if w == "gta":
            out += ["grand", "theft", "auto"]
        elif w in ROMAN and out:
            out.append(str(ROMAN.index(w) + 1))
        else:
            out.append(w)
    return out


def watch_category(url, games_list, every_s=180):
    """Confere a categoria a cada `cada` s; fora do jogo a analise pausa (a live continua gravando)."""
    while not STOP.is_set():
        try:
            r = subprocess.run([SL, "--json", url], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
            v = json.loads(r.stdout or "{}") or {}
            meta = v.get("metadata")
            if not meta and not v.get("streams"):
                # consulta falhou (rede, streamlink, live caiu): mantem o estado anterior. Antes meta=None
                # liberava a analise e um titulo com emoji (decodificado em cp1252) ja derrubava a consulta.
                log("categoria: consulta sem resposta, mantendo", CATEGORY.get("nome") or "o estado anterior")
                STOP.wait(every_s)
                continue
            ok, stream_name = category_matches(meta, games_list)
            if ok != CATEGORY["ok"]:
                log(f"categoria: {stream_name or '?'} -> " + ("analisando" if ok else "fora do jogo/reprise: analise pausada"))
            CATEGORY.update(ok=ok, stream_name=stream_name)
        except Exception as e:
            log("categoria: falhou", e)
        STOP.wait(every_s)


class FinishQueue:
    """Uma thread que finaliza clipes em ordem (whisper e pesado: nao rodar dois ao mesmo tempo)."""

    def __init__(self):
        import queue
        self.q = queue.Queue()
        self.t = threading.Thread(target=self._loop, daemon=True)
        self.t.start()

    def _loop(self):
        while True:
            job = self.q.get()
            if job is None:
                break
            try:
                finish_clip(*job)
            except Exception as e:
                log("  finalizar falhou:", e)

    def close_queue(self):
        self.q.put(None)
        self.t.join()


# ---------------- principal ----------------

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("url")
    ap.add_argument("out")
    ap.add_argument("--minutes", type=float, default=0, help="0 = ate a live acabar / Ctrl+C")
    ap.add_argument("--seg", type=int, default=30)
    ap.add_argument("--window", type=int, default=240, help="segundos de contexto por analise")
    ap.add_argument("--threshold", type=float, default=1.5,
                    help="nota minima do pico (testes 11/2026: picos reais de chat ficaram em 1.9-3.2)")
    ap.add_argument("--min", type=float, default=20)
    ap.add_argument("--max", type=float, default=60)
    ap.add_argument("--pre", type=float, default=30, help="max. segundos de contexto antes do pico")
    ap.add_argument("--post", type=float, default=20, help="max. segundos de desfecho depois do pico")
    ap.add_argument("--lang", default="auto")
    ap.add_argument("--captions", action="store_true", help="queima a legenda do whisper no 9:16")
    ap.add_argument("--chat-offset", type=float, default=2.0, help="segundos que o chat atrasa vs o video")
    ap.add_argument("--quality", default="720p60,720p,best")
    ap.add_argument("--no-chat", action="store_true")
    ap.add_argument("--lazy", action="store_true", help="so corte bruto; whisper/9:16 depois (clip_finish.py)")
    ap.add_argument("--games", default="", help="jogos aceitos, separados por | (categoria da live)")
    a = ap.parse_args()
    global CAPTIONS, LAZY
    CAPTIONS = a.captions
    LAZY = a.lazy
    games_list = [j.strip() for j in a.games.split("|") if j.strip()]

    plat, stream_name = platform_of(a.url)
    buf, clips = os.path.join(a.out, "buf"), os.path.join(a.out, "clips")
    # buf de uma captura morta (app fechado/cancelado) tem segs.csv velho: seria lido como se fosse desta live
    shutil.rmtree(buf, ignore_errors=True)
    os.makedirs(buf, exist_ok=True)
    os.makedirs(clips, exist_ok=True)
    log(f"live {plat}:{stream_name} -> {a.out}")

    chat = Chat(os.path.join(a.out, "chat.jsonl"))
    if not a.no_chat:
        target_fn = {"twitch": (chat_twitch, stream_name), "kick": (chat_kick, stream_name), "youtube": (chat_youtube, a.url)}.get(plat)
        if target_fn:
            threading.Thread(target=target_fn[0], args=(target_fn[1], chat), daemon=True).start()
        else:
            log("chat: plataforma sem coletor; seguindo so com audio/movimento")

    try:
        sl, ff = start_capture(a.url, buf, a.seg, a.quality, plat)
    except (yt_pace.Blocked, yt_pace.Exhausted) as e:
        log("nao vou abrir a live agora:", e)
        STOP.set()
        sys.exit(2)
    signal.signal(signal.SIGINT, lambda *_: STOP.set())
    end_t = time.time() + a.minutes * 60 if a.minutes > 0 else None

    seen_n, seg_times = 0, []          # tempos[i] = (arquivo, inicio no video, fim no video)
    delta = None                    # hora unix - tempo de video (estimado ao fechar cada segmento)
    # pasta reaberta (a live caiu e voltou no mesmo minuto): continua a numeracao em vez de sobrescrever clip_01
    n_clip = len([f for f in os.listdir(clips) if re.fullmatch(r"clip_\d+\.json", f)])
    n_first = n_clip
    cut_spans, last_cut = [], -1e9
    extracted = {}          # segmento -> audio/movimento ja decodificados (cada .ts e lido uma vez so)
    queue_ = FinishQueue()
    last_seg = time.time()
    chat_down = [False]
    threading.Thread(target=watch_category, args=(a.url, games_list), daemon=True).start()
    while not STOP.is_set():
        if end_t and time.time() > end_t:
            log("tempo esgotado")
            break
        if ff.poll() is not None:
            err = log_tail(buf)
            log("captura terminou:", err[-1] if err else f"ffmpeg saiu ({ff.returncode})")
            break
        segs = read_segments(buf)
        if len(segs) == seen_n:
            # sem segmento novo por muito tempo = captura presa: encerra limpo (o vigia do tkd reabre se a live
            # continuar) em vez de ficar parado para sempre
            if time.time() - last_seg > max(3 * a.seg, 120):
                err = log_tail(buf)
                log(f"captura parada ha {time.time() - last_seg:.0f}s sem segmento novo; encerrando",
                    "|", err[-1] if err else "")
                break
            time.sleep(1)
            continue
        last_seg = time.time()
        now_t = time.time()
        for seg_file, dur in segs[seen_n:]:
            seg_start = seg_times[-1][2] if seg_times else 0.0
            seg_times.append((os.path.join(buf, seg_file), seg_start, seg_start + dur))
        seen_n = len(segs)
        delta_new = now_t - seg_times[-1][2]
        delta = delta_new if delta is None else 0.7 * delta + 0.3 * delta_new
        t_fim = seg_times[-1][2]
        log(f"segmento {seen_n} ({t_fim:.0f}s gravados) | chat {chat.total} msgs"
            + (f" | erro chat: {chat.error_msg}" if chat.error_msg else ""))

        # buffer rotativo: guarda so o necessario para a janela
        keep_segs = [t for t in seg_times if t[2] > t_fim - a.window - a.seg]
        for t in seg_times:
            if t not in keep_segs:
                extracted.pop(t[0], None)
                if os.path.exists(t[0]):
                    os.remove(t[0])
        seg_times = keep_segs
        if not CATEGORY["ok"]:
            continue                       # outro jogo ou reprise: nem decodifica (economiza CPU)
        for t in seg_times:
            if t[0] not in extracted:
                try:
                    extracted[t[0]] = M.extract(t[0])
                except Exception as e:
                    log("extracao falhou:", os.path.basename(t[0]), e)
        if t_fim < 90 or len(seg_times) < 2:
            continue                       # aquecendo: base do chat/audio precisa de contexto

        parts_ = [(t[1], extracted[t[0]]) for t in seg_times if t[0] in extracted]
        if len(parts_) < 2:
            continue
        j0 = seg_times[0][1]
        if parts_[0][0] > j0:                      # 1o segmento nao extraiu: preenche com silencio/quadro parado
            parts_.insert(0, (j0, {"dur": parts_[0][0] - j0, "audio": None, "motion": None}))
        raw_j = M.join_raw(parts_)
        window_ts = os.path.join(buf, "janela.ts")   # so e montada se for cortar
        msgs = chat.entre(j0 + delta - 30, t_fim + delta + 30)
        is_down = chat.is_down()
        if is_down != chat_down[0]:
            log("chat fora do ar: notas so com audio/movimento" if is_down else "chat de volta: notas com o chat")
            chat_down[0] = is_down
        meta = {"chat": [] if is_down else [[w - delta - j0, m] for w, m in msgs], "chat_offset": a.chat_offset}
        try:
            # live: contexto antes do pico (6..pre s) e desfecho depois (3..post s); a janela anda enquanto
            # o momento continua forte, entao o clipe vai de --min ate --max conforme a duracao da reacao
            r = M.analyze(window_ts, meta, None, "live", None, (), top=4, mn=a.min, mx=a.max,
                          pre=(6, a.pre), post=(3, a.post), raw=raw_j)
        except Exception as e:
            log("analise falhou:", e)
            continue
        dur_j = t_fim - j0
        # pico novo: ja tem desfecho gravado (>= 10 s depois) e nao foi cortado antes
        cands = [m for m in r["moments"]
                 if dur_j - 90 <= m["peak"] <= dur_j - 10
                 and not any(c0 <= j0 + m["peak"] <= c1 for c0, c1 in cut_spans)]
        best_m = max(cands, key=lambda m: m["score"], default=None)
        if best_m:
            log(f"  melhor pico {j0 + best_m['peak']:.0f}s nota {best_m['score']:+.2f} {best_m['breakdown']}")
        if not best_m or best_m["score"] < a.threshold or t_fim - last_cut < 20:
            continue

        s, e = best_m["start"], min(best_m["end"], dur_j - 0.5)
        # nao repetir trecho de clipe anterior: comeca depois do fim dele
        for c0, c1 in cut_spans:
            if j0 + s < c1 and j0 + e > c0:
                s = max(s, c1 - j0 + 0.2)
        if e - s < a.min:
            log(f"  pico {j0 + best_m['peak']:.0f}s sobrepoe clipe anterior; pulado")
            cut_spans.append((j0 + best_m["peak"] - 1, j0 + best_m["peak"] + 1))
            continue
        n_clip += 1
        base = os.path.join(clips, f"clip_{n_clip:02d}")
        # pro Claude: sem comando de bot (!sorteio) nem linha de controle (\x01 do /me, emote vazio)
        chat_clip = [m for w, m in msgs if j0 + s <= w - delta - a.chat_offset <= j0 + e
                     and "".join(c for c in m if c.isprintable()).strip()[:1] not in ("", "!")]
        info = {"platform": plat, "channel": stream_name, "url": a.url, "score": best_m["score"],
                "breakdown": best_m["breakdown"], "weights": r["weights"],
                "video_start": round(j0 + s, 2), "video_end": round(j0 + e, 2),
                "wall_start": round(j0 + s + delta, 1), "duration": round(e - s, 2),
                "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
                # categoria da live na hora do corte: o Claude confere o jogo (GTA V e GTA VI parecem iguais nos quadros)
                "category": CATEGORY.get("nome")}
        try:
            # so os segmentos que cobrem o trecho (antes: a janela inteira, ~4 min copiados e decodificados
            # ate o inicio do corte, a cada clipe)
            use_segs = [t for t in seg_times if t[2] > j0 + s - 1 and t[1] < j0 + e] or seg_times
            off = use_segs[0][1] - j0
            if not concat([t[0] for t in use_segs], window_ts):
                raise RuntimeError("concat falhou")
            raw = cut_raw(window_ts, s - off, e - off, base)
            cut_spans.append((j0 + s, j0 + e))
            last_cut = t_fim
            queue_.q.put((raw, base, a.lang, chat_clip, info))
            log(f"  CLIP {n_clip}: {e - s:.0f}s nota {best_m['score']:+.2f} | chat no trecho {len(chat_clip)} "
                f"-> {base}.mp4 (9:16 + whisper em segundo plano)")
        except Exception as ex:
            log("  corte falhou:", ex)

    STOP.set()
    for p in (ff, sl):
        if p.poll() is None:
            p.terminate()
    log("finalizando clipes pendentes...")
    queue_.close_queue()
    log(f"fim: {n_clip - n_first} clipes em {clips}")
    if os.environ.get("TKD_LIVE_KEEP_BUF") != "1":
        shutil.rmtree(buf, ignore_errors=True)


if __name__ == "__main__":
    main()
