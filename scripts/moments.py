"""
Momentos fortes de um video-fonte (footage): sinais por segundo -> nota combinada -> picos -> janelas.

uso:
  python scripts/moments.py VIDEO.mp4 OUT.json [--meta INFO.json] [--speech SPEECH.json]
                            [--profile fight|speech] [--weights '{"audio":0.3}'] [--lexicon "frase|nome"]
                            [--top 8] [--min 4] [--max 20] [--transcribe auto|en|ja|pt]

Sinais (todos opcionais, entram so os que existem):
  audio     salto de volume vs base local + fluxo espectral + energia na banda de voz
  motion    diferenca entre quadros (4 fps, 64x36 cinza) + densidade de cortes
  speech    palavras por segundo (falas do speech.py)
  lexicon   frases de efeito / nome do personagem nas falas
  heatmap   "mais repetidos" do YouTube (info.json do yt-dlp)
  comments  timestamps citados nos comentarios do YouTube ("2:13 essa cena"), peso por likes
  chat      chat de live/VOD (meta.chat = [[segundo, msg], ...]): velocidade vs base, emotes/hype, "clip it"

Cada sinal: z robusto (mediana/MAD) -> suavizado -> soma com pesos renormalizados nos sinais presentes.
Saida: {"duration","weights","curve":[nota por segundo],"moments":[{"start","end","peak","score","breakdown"}]}
FFmpeg: TKD_FFMPEG (padrao "ffmpeg"); ffprobe ao lado dele.
"""
import argparse, collections, json, math, os, re, subprocess, sys

import numpy as np
from scipy.ndimage import gaussian_filter1d, median_filter
from scipy.signal import find_peaks

FF = os.environ.get("TKD_FFMPEG", "ffmpeg")
SR = 16000

PROFILES = {
    "fight": {"audio": 0.30, "motion": 0.30, "comments": 0.20, "heatmap": 0.15, "speech": 0.05, "chat": 0.20},
    "speech": {"speech": 0.25, "lexicon": 0.25, "comments": 0.20, "heatmap": 0.15, "audio": 0.15, "chat": 0.20},
    # live/stream: o chat e o sinal mais forte (reacao do publico em tempo real)
    "live": {"chat": 0.40, "audio": 0.25, "speech": 0.12, "motion": 0.08, "lexicon": 0.05, "heatmap": 0.10},
}

# ---------- chat (Twitch / Kick / YouTube live) ----------
# reacoes do chat: emotes globais + gírias en/pt-BR. Token inteiro, minusculo.
HYPE = {
    "kekw", "lul", "lulw", "omegalul", "icant", "lmao", "lmfao", "lol", "xd", "pog", "pogchamp", "poggers", "pogu",
    "pagman", "sheesh", "letsgo", "insane", "holy", "omg", "wtf", "monkas", "monkaw", "pepelaugh", "kekl", "w", "ww",
    "www", "l", "f", "rip", "sadge", "gg", "ez", "clutch", "aaaa", "huh",
    "kkkk", "kkkkk", "kkkkkk", "kkkkkkk", "rsrs", "mds", "caralho", "carai", "pqp", "slk", "socorro", "misericordia",
    "nossa", "brabo", "monstro", "absurdo", "vish", "eita", "uau",
}
HYPE_EMOJI = ("😂", "🤣", "💀", "🔥", "😱", "😭", "🤯", "😳")
CLIP_RE = re.compile(r"\b(clip+(\s*(it|that|isso|ai|aí))?|clipa|clipou|clipped)\b", re.IGNORECASE)
TOK_RE = re.compile(r"[\w']+|\?+", re.UNICODE)


def hype_of(msg):
    toks = [t.lower() for t in TOK_RE.findall(msg)]
    if not toks:
        return 0.0
    hits = sum(1 for t in toks if t in HYPE or re.fullmatch(r"k{4,}|(ha){3,}|(ja){3,}|(rs){2,}", t))
    hits += sum(msg.count(e) for e in HYPE_EMOJI)
    letters = [c for c in msg if c.isalpha()]
    caps = sum(c.isupper() for c in letters) / len(letters) if len(letters) >= 4 else 0.0
    q = 1.0 if re.search(r"\?{3,}", msg) else 0.0
    return min(3.0, 0.6 * hits + 0.8 * caps + q)


# emote de risada/espanto do proprio canal (alanzKEKW, davyLUL...): conta como reacao mesmo repetido
REACT_SUB = re.compile(r"kek|lul|lol|laugh|xd|haha|pog|omg|wtf|monka|icant|kk", re.IGNORECASE)
SPAM_WIN_S = 30.0
SPAM_REPEAT = 3


def norm_msg(msg):
    """Forma canonica para achar repeticao: minusculas, sem pontuacao, letras repetidas e palavras repetidas
    colapsadas ("EUUU!!" -> "eu", "alanzamor alanzamor alanzamor" -> "alanzamor")."""
    m = re.sub(r"[^\w\s]", "", (msg or "").lower(), flags=re.UNICODE)
    m = re.sub(r"(\w)\1{2,}", r"\1", m)
    out = []
    for w in m.split():
        if not out or out[-1] != w:
            out.append(w)
    return " ".join(out)


def is_reaction(msg):
    """Risada, espanto ou emote de reacao: o que o chat manda quando algo acontece na tela."""
    toks = [t.lower() for t in TOK_RE.findall(msg)]
    if any(t in HYPE or re.fullmatch(r"k{4,}|(ha){3,}|(ja){3,}|(rs){2,}", t) for t in toks):
        return True
    return any(e in msg for e in HYPE_EMOJI) or bool(REACT_SUB.search(msg))


def chat_weights(chat):
    """Peso de cada mensagem nos sinais do chat (1 = conta, 0 = participacao).
    Sorteio/comando/comemoracao lota o chat sem nada acontecer na tela: "!sorteio", "eu", "gta6", emote do canal
    em fila. Medido em 18/11: esses picos viravam corte (nota 2-3.5 no Claude, "sorteio de sub", "300k").
    Comando (`!x`) e linha vazia/controle nunca contam; texto que nao e reacao e ja apareceu SPAM_REPEAT vezes nos
    ultimos SPAM_WIN_S segundos para de contar (a primeira onda conta, a fila de copias nao)."""
    seen = collections.deque()
    counts = collections.Counter()
    w = np.ones(len(chat))
    for i, (t, msg) in enumerate(chat):
        t = float(t)
        txt = "".join(c for c in (msg or "") if c.isprintable()).strip()
        if not txt or txt.startswith("!"):
            w[i] = 0.0
            continue
        key = norm_msg(txt)
        while seen and seen[0][0] < t - SPAM_WIN_S:
            counts[seen.popleft()[1]] -= 1
        if not is_reaction(txt) and counts[key] >= SPAM_REPEAT:
            w[i] = 0.0
        seen.append((t, key))
        counts[key] += 1
    return w


def sig_chat(chat, n, offset=4.0, base_win=121):
    """chat = [(segundo no video, mensagem)]. O chat atrasa ~3-6 s em relacao ao video: recua `offset`.
    Devolve z combinado de: velocidade vs base local (surpresa ~Poisson), hype/emotes e pedidos de "clip".
    Mensagem de participacao (sorteio, comando, copia em fila) pesa 0 (`chat_weights`)."""
    if len(chat) < 20:
        return None, {}
    chat = sorted(chat, key=lambda x: float(x[0]))
    t = np.array([max(0.0, float(a) - offset) for a, _ in chat])
    wt = chat_weights(chat)
    cnt = per_second(wt, t, n, "sum")
    base = local_base(cnt, base_win)
    vel = (cnt - base) / np.sqrt(base + 1.0)
    hype = per_second(wt * np.array([hype_of(m) for _, m in chat]), t, n, "sum")
    clip = per_second(np.array([1.0 if CLIP_RE.search(m) else 0.0 for _, m in chat]), t, n, "sum")
    z = 0.55 * rz(soft(vel, 1.0)) + 0.30 * rz(soft(hype, 1.0)) + 0.15 * sparse(soft(clip, 1.5))
    return z, {"chat_msgs": len(chat), "chat_clip_mentions": int(clip.sum())}


def ffprobe_bin():
    d, b = os.path.split(FF)
    return os.path.join(d, b.replace("ffmpeg", "ffprobe")) if d else b.replace("ffmpeg", "ffprobe")


def duration_of(path):
    r = subprocess.run([ffprobe_bin(), "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    try:
        return float(r.stdout.strip().splitlines()[0])
    except Exception:
        return 0.0


# ---------- normalizacao ----------

def rz(x, lo=-2.0, hi=8.0):
    """z robusto: centro na mediana, escala no MAD (cai para desvio padrao se o sinal e quase todo zero)."""
    x = np.asarray(x, dtype=np.float64)
    if x.size == 0:
        return x
    med = np.median(x)
    mad = 1.4826 * np.median(np.abs(x - med))
    sd = x.std()
    scale = mad if mad > 1e-9 else (sd if sd > 1e-9 else 1.0)
    return np.clip((x - med) / scale, lo, hi)


def sparse(x, top=3.0):
    """sinal de eventos (quase todo zero): z nao serve (MAD=0). Escala pelo p99 dos nao-zero -> 0..top."""
    x = np.asarray(x, dtype=np.float64)
    nz = x[x > 1e-9]
    if nz.size == 0:
        return np.zeros_like(x)
    ref = np.percentile(nz, 99) or nz.max()
    return np.clip(x / ref * top, 0, top)


def soft(x, sigma):
    return gaussian_filter1d(np.asarray(x, dtype=np.float64), sigma) if sigma > 0 and len(x) > 2 else np.asarray(x, float)


def local_base(x, win):
    win = int(max(3, min(win, len(x) - (1 - len(x) % 2))))
    win += 1 - win % 2
    return median_filter(np.asarray(x, dtype=np.float64), size=win, mode="nearest")


def per_second(values, times, n, how="mean"):
    out = np.zeros(n)
    if len(values) == 0:
        return out
    idx = np.clip(np.floor(times).astype(int), 0, n - 1)
    sums = np.bincount(idx, weights=values, minlength=n)[:n]
    if how == "sum":
        return sums
    cnt = np.bincount(idx, minlength=n)[:n]
    return np.where(cnt > 0, sums / np.maximum(cnt, 1), 0.0)


# ---------- sinais ----------

def raw_audio(path):
    """Decodifica o audio uma vez: series por quadro (hop 32 ms) com tempo em segundos.
    Separado da pontuacao para a live reaproveitar cada segmento (sem redecodificar a janela toda)."""
    raw = subprocess.run([FF, "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", str(SR), "-f", "f32le", "-"],
                         capture_output=True).stdout
    x = np.frombuffer(raw, dtype=np.float32)
    if x.size < SR:
        return None
    frame, hop = 1024, 512
    win = np.hanning(frame).astype(np.float32)
    freqs = np.fft.rfftfreq(frame, 1 / SR)
    vband = (freqs >= 250) & (freqs <= 3500)
    e_v, flux_v, voice_v, t_v = [], [], [], []
    prev = None
    block = SR * 30                                   # 30 s por vez: memoria constante em video longo
    for a in range(0, x.size - frame, block):
        seg = x[a:a + block + frame]
        nfr = 1 + (seg.size - frame) // hop
        if nfr <= 0:
            break
        idx = np.arange(frame)[None, :] + hop * np.arange(nfr)[:, None]
        fr = seg[idx]
        mag = np.abs(np.fft.rfft(fr * win, axis=1))
        lm = np.log1p(10 * mag)
        stack = lm if prev is None else np.vstack([prev, lm])
        d = np.maximum(np.diff(stack, axis=0), 0).sum(axis=1)
        if prev is None:
            d = np.concatenate([[0.0], d])
        prev = lm[-1:]
        tot = mag.sum(axis=1) + 1e-9
        e_v.append((fr ** 2).mean(axis=1) + 1e-12)
        flux_v.append(d)
        voice_v.append(mag[:, vband].sum(axis=1) / tot)
        t_v.append((a + hop * np.arange(nfr) + frame / 2) / SR)
    return {"t": np.concatenate(t_v), "e": np.concatenate(e_v), "flux": np.concatenate(flux_v),
            "voice": np.concatenate(voice_v)}


def audio_from_raw(r, n):
    if r is None or not len(r["t"]):
        return None, None
    loud = 10 * np.log10(per_second(r["e"], r["t"], n) + 1e-10)
    flux = per_second(r["flux"], r["t"], n)
    voice = per_second(r["voice"], r["t"], n)
    jump = loud - local_base(loud, 61)
    yell = np.maximum(jump, 0) * np.clip(voice, 0, 1)
    return 0.5 * rz(jump) + 0.3 * rz(flux) + 0.2 * rz(yell), loud


def sig_audio(path, n):
    return audio_from_raw(raw_audio(path), n)


MOTION_FPS = 4


def raw_motion(path, fps=MOTION_FPS):
    """Diferenca media entre quadros (64x36 cinza) com tempo em segundos. So CPU."""
    w, h = 64, 36
    # decodificacao so na CPU: hwaccel (d3d12va/vulkan) derrubou o driver AMD e reiniciou o PC em 16/11/2026
    raw = subprocess.run([FF, "-v", "error", "-i", path, "-an",
                          "-vf", f"fps={fps},scale={w}:{h}:flags=area,format=gray", "-f", "rawvideo", "-"],
                         capture_output=True).stdout
    fr = np.frombuffer(raw, dtype=np.uint8)
    k = fr.size // (w * h)
    if k < 3:
        return None
    fr = fr[: k * w * h].reshape(k, h, w).astype(np.float32)
    return {"t": (np.arange(1, k) + 0.5) / fps, "d": np.abs(np.diff(fr, axis=0)).mean(axis=(1, 2))}


def motion_from_raw(r, n, fps=MOTION_FPS):
    if r is None or len(r["d"]) < 3:
        return None, []
    d, t = r["d"], r["t"]
    # corte = salto bem acima da vizinhanca (+-2 s); limiar global falha em AMV/luta, onde tudo mexe
    loc = median_filter(d, size=2 * 2 * fps + 1, mode="nearest")
    is_cut = (d > np.maximum(18.0, 2.5 * loc)) & (d >= np.maximum(np.roll(d, 1), np.roll(d, -1)))
    cuts = t[is_cut]
    diff = per_second(np.where(is_cut, loc, d), t, n)    # corte nao conta como "movimento"
    density = soft(per_second(is_cut.astype(float), t, n, "sum"), 1.5)
    return 0.6 * rz(diff) + 0.4 * rz(density), [round(float(c), 2) for c in cuts]


def sig_motion(path, n, fps=MOTION_FPS):
    return motion_from_raw(raw_motion(path, fps), n, fps)


def extract(path):
    """Tudo que exige decodificar o arquivo, uma vez."""
    return {"dur": duration_of(path), "audio": raw_audio(path), "motion": raw_motion(path)}


def join_raw(parts):
    """Junta extracoes de segmentos consecutivos: [(inicio_em_s, extract(...)), ...] -> extracao unica,
    tempos relativos ao inicio do primeiro."""
    if not parts:
        return None
    t0 = parts[0][0]
    out = {"dur": parts[-1][0] + parts[-1][1]["dur"] - t0, "audio": None, "motion": None}
    for key_name, campos in (("audio", ("e", "flux", "voice")), ("motion", ("d",))):
        ok = [(seg_start, r[key_name]) for seg_start, r in parts if r.get(key_name) is not None]
        if ok:
            out[key_name] = {"t": np.concatenate([r["t"] + (seg_start - t0) for seg_start, r in ok]),
                          **{c: np.concatenate([r[c] for _, r in ok]) for c in campos}}
    return out


def load_words(speech):
    """aceita a saida do speech.py `file` ({lines}) ou `segments` ({segments:[{lines}]})."""
    lines = speech.get("lines")
    if lines is None:
        lines = [ln for sg in speech.get("segments", []) for ln in sg.get("lines", [])]
    return [w for ln in lines for w in ln.get("words", [])]


def norm_txt(s):
    return re.sub(r"[^\w' ]+", " ", s.lower()).split()


def sig_speech(words, n, lexicon):
    if not words:
        return None, None
    mids = np.array([(w["s"] + w["e"]) / 2 for w in words])
    rate = soft(per_second(np.ones_like(mids), mids, n, "sum"), 1.0)
    lex = None
    phrases = [norm_txt(p) for p in lexicon if p.strip()]
    if phrases:
        toks = [" ".join(norm_txt(w["w"])) for w in words]
        hits = np.zeros(n)
        for i in range(len(words)):
            for ph in phrases:
                L = len(ph)
                if L and toks[i:i + L] == ph:
                    hits[min(n - 1, int(mids[i]))] += 1.0 + 0.5 * (L - 1)
        lex = sparse(soft(hits, 1.5)) if hits.any() else None
    return rz(rate), lex


def sig_heatmap(meta, n):
    hm = meta.get("heatmap") or []
    if not hm:
        return None
    out = np.zeros(n)
    for b in hm:
        a, e = int(b.get("start_time", 0)), int(math.ceil(b.get("end_time", 0)))
        out[max(0, a):min(n, max(a + 1, e))] = float(b.get("value", 0))
    return rz(out) if out.any() else None


TS = re.compile(r"(?<![\d:])(?:(\d{1,2}):)?(\d{1,2}):([0-5]\d)(?![\d:])")


def sig_comments(meta, n):
    ev = np.zeros(n)
    hits = 0
    for c in meta.get("comments") or []:
        txt = c.get("text") or ""
        stamps = {int(h or 0) * 3600 + int(m) * 60 + int(s) for h, m, s in TS.findall(txt)}
        if not stamps or len(stamps) > 4:            # lista de capitulos nao e reacao
            continue
        wgt = 1.0 + math.log1p(max(0, int(c.get("like_count") or 0)))
        for t in stamps:
            if 0 <= t < n:
                ev[t] += wgt / len(stamps)
                hits += 1
    if hits < 2:
        return None, 0
    return sparse(soft(ev, 2.0)), hits


# ---------- momentos ----------

def boundaries(words, gap=0.35):
    st, en = [], []
    for i, w in enumerate(words):
        p = words[i - 1] if i else None
        q = words[i + 1] if i + 1 < len(words) else None
        if p is None or w["s"] - p["e"] >= gap:
            st.append(w["s"] - 0.15)
        if q is None or q["s"] - w["e"] >= gap:
            en.append(w["e"] + 0.25)
    return st, en


def snap(t, pts, win, earlier):
    c = [p for p in pts if abs(p - t) <= win]
    if not c:
        return None
    return min(c, key=lambda p: abs(p - t) + (0.2 if (p > t) == earlier else 0))


def inside(t, words):
    for w in words:
        if w["s"] < t < w["e"]:
            return w
    return None


def quietest(t, loud, win=1.5):
    if loud is None:
        return t
    a, b = max(0, int(t - win)), min(len(loud), int(t + win) + 1)
    return t if b <= a else float(a + int(np.argmin(loud[a:b])) + 0.5)


def window(peak, curve, words, loud, dur, mn, mx, pre=(2, 8), post=(1, 4)):
    n = len(curve)
    pv = float(curve[peak])
    floor = float(np.median(curve))
    thr = floor + 0.35 * (pv - floor)
    i = peak
    while i > 0 and curve[i - 1] >= thr and peak - i < pre[1]:
        i -= 1
    j = peak
    while j < n - 1 and curve[j + 1] >= thr and j - peak < post[1]:
        j += 1
    s = float(np.clip(i - 1, peak - pre[1], peak - pre[0]))
    e = float(np.clip(j + 1, peak + post[0], peak + post[1]))
    if e - s < mn:
        need = mn - (e - s)
        s, e = s - 0.6 * need, e + 0.4 * need
    near = [w for w in words if s - 6 <= w["s"] <= e + 6]
    st, en = boundaries(near)
    ss = snap(s, st, 2.5, True)
    if ss is None:
        w = inside(s, near)
        ss = w["s"] - 0.1 if w else quietest(s, loud)
    ee = snap(e, en, 2.5, False)
    if ee is None:
        w = inside(e, near)
        ee = w["e"] + 0.2 if w else quietest(e, loud)
    s, e = ss, ee
    if e - s > mx:
        s = e - mx
    if e - s < mn:
        e = s + mn
    s, e = max(0.0, s), min(dur, e)
    seg = curve[int(s):max(int(s) + 1, int(math.ceil(e)))]
    top = np.sort(seg)[-max(1, len(seg) // 3):]
    score = 0.55 * pv + 0.45 * float(top.mean())
    return {"start": round(s, 2), "end": round(e, 2), "peak": int(peak), "score": round(score, 3)}


def nms(cands, top, gap=0.5):
    keep = []
    for c in sorted(cands, key=lambda c: -c["score"]):
        if all(c["end"] + gap <= k["start"] or c["start"] >= k["end"] + gap for k in keep):
            keep.append(c)
        if len(keep) >= top:
            break
    return sorted(keep, key=lambda c: -c["score"])


def analyze(video, meta=None, speech=None, profile="fight", weights=None, lexicon=(), top=8, mn=4.0, mx=20.0,
            transcribe=None, pre=(2, 8), post=(1, 4), raw=None):
    """pre/post = (min, max) segundos antes/depois do pico. Footage de edit: janela curta (2-8 / 1-4).
    Live: contexto + desfecho (ex.: 6-30 / 3-20) para clipes de ate 60 s.
    raw = extracao pronta (extract/join_raw); sem ela decodifica `video` aqui."""
    raw = raw if raw is not None else extract(video)
    dur = raw["dur"]
    n = max(1, int(math.ceil(dur)))
    meta = meta or {}
    if speech is None and transcribe:
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import speech as sp  # noqa: E402
        speech = sp.transcribe(video, transcribe)
    words = load_words(speech) if speech else []

    z, info = {}, {}
    a, loud = audio_from_raw(raw.get("audio"), n)
    if a is not None:
        z["audio"] = a
    m, cuts = motion_from_raw(raw.get("motion"), n)
    if m is not None:
        z["motion"] = m
        info["cuts"] = len(cuts)
    sp_rate, lex = sig_speech(words, n, list(lexicon))
    if sp_rate is not None:
        z["speech"] = sp_rate
    if lex is not None:
        z["lexicon"] = lex
    hm = sig_heatmap(meta, n)
    if hm is not None:
        z["heatmap"] = hm
    cm, hits = sig_comments(meta, n)
    if cm is not None:
        z["comments"] = cm
        info["comment_stamps"] = hits
    if meta.get("chat"):
        ch, ci = sig_chat(meta["chat"], n, float(meta.get("chat_offset", 4.0)))
        if ch is not None:
            z["chat"] = ch
            info.update(ci)
    z = {k: soft(v[:n], 2.0) for k, v in z.items()}

    w = dict(PROFILES.get(profile, PROFILES["fight"]))
    w.update(weights or {})
    w = {k: v for k, v in w.items() if k in z and v > 0} or {k: 1.0 for k in z}
    tot = sum(w.values()) or 1.0
    w = {k: v / tot for k, v in w.items()}
    curve = np.zeros(n)
    for k, v in w.items():
        curve += v * z[k]
    if profile == "speech" and words:
        pres = soft(per_second(np.ones(len(words)), np.array([(x["s"] + x["e"]) / 2 for x in words]), n, "sum"), 4.0)
        f = 0.25 + 0.75 * np.clip(pres / 0.8, 0, 1)
        curve = np.where(curve > 0, curve * f, curve)

    moments = []
    if n >= mn:
        pk, _ = find_peaks(curve, distance=max(3, int(mn)), prominence=0.05)
        if not len(pk):
            pk = np.array([int(np.argmax(curve))])
        pk = pk[np.argsort(-curve[pk])][: top * 4]
        cands = [window(int(p), curve, words, loud, dur, mn, mx, tuple(pre), tuple(post)) for p in pk]
        moments = nms(cands, top)
        for c in moments:
            a_, b_ = int(c["start"]), max(int(c["start"]) + 1, int(math.ceil(c["end"])))
            c["breakdown"] = {k: round(float(v * z[k][a_:b_].mean()), 3) for k, v in w.items()}
    return {"duration": round(dur, 2), "profile": profile, "weights": {k: round(v, 3) for k, v in w.items()},
            "signals": info, "curve": [round(float(v), 3) for v in curve], "moments": moments}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("video")
    ap.add_argument("out")
    ap.add_argument("--meta")
    ap.add_argument("--speech")
    ap.add_argument("--profile", default="fight", choices=sorted(PROFILES))
    ap.add_argument("--weights", default="")
    ap.add_argument("--lexicon", default="")
    ap.add_argument("--top", type=int, default=8)
    ap.add_argument("--min", type=float, default=4.0)
    ap.add_argument("--max", type=float, default=20.0)
    ap.add_argument("--transcribe", default="", help="sem --speech: transcreve com speech.py neste idioma")
    a = ap.parse_args()
    load = lambda p: json.load(open(p, encoding="utf-8")) if p and os.path.exists(p) else None  # noqa: E731
    r = analyze(a.video, load(a.meta), load(a.speech), a.profile, json.loads(a.weights) if a.weights else None,
                [x for x in a.lexicon.split("|") if x.strip()], a.top, a.min, a.max, a.transcribe or None)
    json.dump(r, open(a.out, "w", encoding="utf-8"), ensure_ascii=False)
    for m in r["moments"]:
        bd = " ".join(f"{k}={v:+.2f}" for k, v in m["breakdown"].items())
        print(f"{m['start']:8.2f}-{m['end']:8.2f}  score {m['score']:+.2f}  {bd}", file=sys.stderr)


if __name__ == "__main__":
    main()
