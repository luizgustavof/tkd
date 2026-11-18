"""
Acesso web anonimo (sem conta) do tkd, via Scrapling. Ver docs/SCRAPLING_TODO.md.

- `http_session()`: sessao HTTP com TLS/HTTP2 de Chrome (curl_cffi, o motor do Fetcher do Scrapling) e a mesma API
  do `requests`. Sem Scrapling instalado cai no `requests` puro.
- `browser_session()`: StealthySession do Scrapling (patchright) no Edge instalado, com perfil anonimo proprio
  (.profiles/_scrapling), separado dos perfis logados dos canais.
- `tiktok_pace()` / `tiktok_wall()`: ritmo das paginas do TikTok, dividido entre processos.
- `args()/emit()/evt()/log()`: mesmo contrato dos scripts de browser/*.mjs (payload em --json, UMA linha JSON no
  stdout, eventos `{"evt":1,...}` e logs no stderr).

Diminui a deteccao como bot, mas nao e garantia: o muro anti-bot ainda aparece e 429 por IP continua valendo.
"""
import json, os, random, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ANON_PROFILE = os.path.join(ROOT, ".profiles", "_scrapling")
EDGE_PATHS = [
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
]


def args():
    if "--json" in sys.argv:
        i = sys.argv.index("--json")
        return json.loads(sys.argv[i + 1] if i + 1 < len(sys.argv) else "{}")
    return {}


def emit(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def evt(**o):
    sys.stderr.write(json.dumps({"evt": 1, **o}, ensure_ascii=False) + "\n")
    sys.stderr.flush()


def log(*a):
    print("[scrapling]", *a, file=sys.stderr, flush=True)


def human(page, a=400, b=1800):
    """Pausa humana dentro do navegador (ms)."""
    page.wait_for_timeout(a + random.randint(0, max(0, b - a)))


def http_session(headers=None):
    """Sessao HTTP com impressao digital TLS de Chrome. API do requests (status_code, json(), cookies.set...).
    Nao sobrescreve o User-Agent: o impersonate ja manda o do Chrome que bate com o TLS."""
    try:
        from curl_cffi import requests as cr
        s = cr.Session(impersonate="chrome", verify=_ascii_ca_path())
    except ImportError:
        import requests
        s = requests.Session()
        s.headers["User-Agent"] = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
                                   "(KHTML, like Gecko) Chrome/140.0 Safari/537.36")
    if headers:
        s.headers.update(headers)
    return s


def _ascii_ca_path():
    """O curl (curl_cffi) nao abre o cacert.pem do certifi num caminho com acento ("Área de Trabalho") quando o
    Python roda em modo UTF-8 (erro 77). Devolve um caminho so ASCII: o nome curto 8.3 do Windows ou uma copia no
    temp. True (verificacao padrao) se o caminho ja e ASCII ou nada der certo."""
    try:
        import certifi
        ca = certifi.where()
    except Exception:
        return True
    if ca.isascii():
        return True
    if os.name == "nt":
        try:
            import ctypes
            buf = ctypes.create_unicode_buffer(1024)
            if ctypes.windll.kernel32.GetShortPathNameW(ca, buf, 1024) and buf.value.isascii():
                return buf.value
        except Exception:
            pass
    import shutil, tempfile
    dst = os.path.join(tempfile.gettempdir(), "tkd-cacert.pem")
    try:
        if not os.path.exists(dst) or os.path.getmtime(dst) < os.path.getmtime(ca):
            shutil.copyfile(ca, dst)
        return dst if dst.isascii() else True
    except Exception:
        return True


def edge_path():
    p = os.environ.get("TKD_EDGE")
    if p and os.path.exists(p):
        return p
    return next((p for p in EDGE_PATHS if os.path.exists(p)), None)


def default_headless():
    """TKD_SCRAPLING_HEADLESS=0 abre a janela (quando o muro aparecer demais no headless)."""
    return os.environ.get("TKD_SCRAPLING_HEADLESS", "1") != "0"


# ------------------------------------------------------------------ ritmo do TikTok
# Em 17/11 ~8 aberturas do perfil em 5 min deram muro suave por IP (perfil abre, grade vazia, item_list 200 vazio).
# Ritmo dividido entre processos por arquivo, como o yt_pace: intervalo minimo, teto por janela e pausa apos muro.
PACE_FILE = os.path.join(ROOT, ".profiles", "_scrapling_pace.json")
# tipo de pagina: (intervalo minimo em s, teto de aberturas por WINDOW). O perfil (grade) foi o que tomou muro.
PACE_KINDS = {"profile": (45, 6), "video": (8, 40)}
WINDOW = 900            # 15 min
WALL_PAUSE = 1800       # s sem abrir nada depois de detectar o muro


class Walled(Exception):
    """TikTok em pausa por muro anti-bot recente (ou ritmo estourado); nao abrir de novo agora."""


def _pace_read():
    try:
        with open(PACE_FILE, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {"opens": {}, "wall_until": 0}


def _pace_write(state):
    os.makedirs(os.path.dirname(PACE_FILE), exist_ok=True)
    tmp = PACE_FILE + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f)
    os.replace(tmp, PACE_FILE)


def tiktok_pace(kind="profile", max_wait=600):
    """Chamar antes de cada pagina do TikTok (inclusive reload). Dorme o necessario (ate max_wait s);
    durante a pausa de muro levanta Walled."""
    interval, cap = PACE_KINDS[kind]
    while True:
        state, now = _pace_read(), time.time()
        if state.get("wall_until", 0) > now:
            raise Walled(f"muro anti-bot recente; TikTok em pausa por mais {int(state['wall_until'] - now)}s")
        opens = state.get("opens") or {}
        recent = [t for t in opens.get(kind, []) if now - t < WINDOW]
        last = max([t for ts in opens.values() for t in ts] or [0])   # qualquer pagina conta para o intervalo
        wait = max(last + interval - now, 0.0)
        if len(recent) >= cap:
            wait = max(wait, recent[-cap] + WINDOW - now)
        if wait <= 0:
            opens = {k: [t for t in ts if now - t < WINDOW] for k, ts in opens.items()}
            opens[kind] = recent + [now]
            state["opens"] = opens
            _pace_write(state)
            return
        if wait > max_wait:
            raise Walled(f"ritmo do TikTok: proxima abertura so em {int(wait)}s")
        log(f"ritmo do TikTok: esperando {wait:.0f}s")
        time.sleep(wait + random.random() * 5)
        max_wait -= wait


def tiktok_wall():
    """Registra o muro: ninguem (scout, comentarios) abre o TikTok de novo por WALL_PAUSE."""
    state = _pace_read()
    state["wall_until"] = time.time() + WALL_PAUSE
    _pace_write(state)
    log(f"muro anti-bot: TikTok em pausa por {WALL_PAUSE // 60} min")


def browser_session(headless=None, record=False, **kw):
    """StealthySession pronta para usar com `with`. Edge instalado (menos cara de automacao que o Chromium baixado),
    canvas com ruido, perfil anonimo persistente (cookies de visitante voltam na proxima vez)."""
    from scrapling.fetchers import StealthySession
    os.makedirs(ANON_PROFILE, exist_ok=True)
    opts = dict(
        headless=default_headless() if headless is None else headless,
        hide_canvas=True,
        locale="pt-BR",
        timezone_id="America/Sao_Paulo",
        user_data_dir=ANON_PROFILE,
        record_requests=record,
        max_recorded_requests=3000,
        timeout=45000,
    )
    exe = edge_path()
    if exe:
        opts["executable_path"] = exe
    opts.update(kw)
    return StealthySession(**opts)


if __name__ == "__main__":
    # python scripts/web.py get URL [Accept-Language]: corpo da pagina publica no stdout (usado pelo browser/lib.mjs)
    if len(sys.argv) >= 3 and sys.argv[1] == "get":
        hdr = {"Accept-Language": sys.argv[3]} if len(sys.argv) > 3 else None
        r = http_session(hdr).get(sys.argv[2], timeout=20)
        sys.stdout.buffer.write(r.content)
        sys.exit(0 if r.status_code < 400 else 1)
    print("uso: web.py get URL [Accept-Language]", file=sys.stderr)
    sys.exit(2)
