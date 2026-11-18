"""
Orcamento de requisicoes ao YouTube por IP, compartilhado com o Rust (crates/tkd-core/src/yt_pace.rs):
out/.yt_pace.json. Vale com ou sem os cookies da conta descartavel (o limite e por IP).

uso em Python:
    import yt_pace
    yt_pace.reserve(yt_pace.WEIGHT_DOWNLOAD)     # espera a vez (ou levanta Blocked / Exhausted)
    ...chama o YouTube...
    if yt_pace.is_block(stderr): yt_pace.mark_blocked()
uso em shell:  python scripts/yt_pace.py reserve 4   (sai 1 se em pausa/esgotado)
               python scripts/yt_pace.py status
"""
import json, os, sys, time

HOURLY_BUDGET = 200   # era 600; 17/11/2026 barrou em 60/600 (igual ao Rust)
WINDOW = 3600
MIN_INTERVAL = 4
PAUSES = [3600, 3 * 3600, 6 * 3600, 12 * 3600]  # cresce se o bloqueio volta logo (igual ao Rust)
STREAK_WINDOW = 3 * 3600
MAX_WAIT = 15 * 60
WEIGHT_SEARCH, WEIGHT_DOWNLOAD, WEIGHT_META, WEIGHT_CHAT = 6, 4, 12, 1
WEIGHT_SEARCH_FLAT = 2  # --flat-playlist: so a pagina de resultados (igual ao yt_pace.rs)
SLEEP_ARGS = ["--sleep-requests", "1", "--sleep-interval", "5", "--max-sleep-interval", "10"]


def cookie_args():
    """["--cookies", arquivo] da conta Google descartavel (`tkd yt-login`), ou [] se nao houver.
    TKD_YT_COOKIES=0 desliga. O Rust reexporta o arquivo quando ele fica velho (footage.rs)."""
    if os.environ.get("TKD_YT_COOKIES") == "0":
        return []
    f = os.path.join(_root(), ".profiles", "_youtube", "cookies.txt")
    return ["--cookies", f] if os.path.exists(f) else []


class Blocked(RuntimeError):
    pass


class Exhausted(RuntimeError):
    pass


def _root():
    return os.environ.get("TKD_ROOT") or os.getcwd()


def _state_file():
    # TKD_YT_PACE_FILE: experimentos de caos usam um arquivo proprio (o mesmo que o Rust)
    return os.environ.get("TKD_YT_PACE_FILE") or os.path.join(_root(), "out", ".yt_pace.json")


def _legacy_state_file():
    """arquivo de antes da troca de nome; lido so enquanto o novo nao existe (preserva uma pausa em andamento)"""
    return os.path.join(_root(), "out", ".yt_ritmo.json")


class _FileLock:
    """Trava entre processos (Rust e Python escrevem o mesmo arquivo): cria .lock exclusivo."""

    def __init__(self):
        self.p = _state_file() + ".lock"

    def __enter__(self):
        os.makedirs(os.path.dirname(self.p), exist_ok=True)
        t0 = time.time()
        while True:
            try:
                fd = os.open(self.p, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.close(fd)
                return self
            except FileExistsError:
                try:
                    if time.time() - os.path.getmtime(self.p) > 10:   # trava esquecida por processo morto
                        os.remove(self.p)
                        continue
                except OSError:
                    pass
                if time.time() - t0 > 10:
                    return self                                       # segue sem trava em vez de travar tudo
                time.sleep(0.05)

    def __exit__(self, *a):
        try:
            os.remove(self.p)
        except OSError:
            pass


def _read():
    f = _state_file() if os.path.exists(_state_file()) else _legacy_state_file()
    try:
        with open(f, encoding="utf-8") as fh:
            d = json.load(fh)
        # chaves antigas em portugues (out/.yt_ritmo.json) ainda aceitas
        return {"calls": d.get("calls", d.get("chamadas", [])),
                "paused_until": d.get("paused_until", d.get("pausa_ate", 0)),
                "streak": d.get("streak", d.get("seguidos", 0))}
    except Exception:
        return {"calls": [], "paused_until": 0, "streak": 0}


def _write(state):
    os.makedirs(os.path.dirname(_state_file()), exist_ok=True)
    tmp = _state_file() + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(state, fh)
    os.replace(tmp, _state_file())


def _clock(t):
    return time.strftime("%H:%M", time.localtime(t))


def reserve(weight, max_wait=MAX_WAIT, interval=MIN_INTERVAL):
    """Espera a vez e reserva `weight` requisicoes. interval=0 para chamadas leves (chat) que ja tem ritmo proprio."""
    t_start = time.time()
    while True:
        with _FileLock():
            s = _read()
            t = int(time.time())
            if t < s["paused_until"]:
                raise Blocked(f"YouTube bloqueou este IP; pausa ate {_clock(s['paused_until'])}")
            s["calls"] = [c for c in s["calls"] if t - c[0] < WINDOW]
            used = sum(c[1] for c in s["calls"])
            last = max((c[0] for c in s["calls"]), default=0)
            if used + weight <= HOURLY_BUDGET and t >= last + interval:
                s["calls"].append([t, weight])
                _write(s)
                return
        if used + weight > HOURLY_BUDGET:
            needed, until = used + weight - HOURLY_BUDGET, t
            for c0, w in sorted(s["calls"]):
                until = c0 + WINDOW
                if w >= needed:
                    break
                needed -= w
            wait = until - t + 1
            if time.time() - t_start + wait > max_wait:
                raise Exhausted(f"orcamento do YouTube esgotado ({used}/{HOURLY_BUDGET}); libera as {_clock(until)}")
        else:
            wait = last + interval - t
        time.sleep(max(0.2, min(wait, 30)))


def is_block(err):
    err = err or ""
    return "429" in err or "Too Many Requests" in err or "not a bot" in err


def mark_blocked():
    with _FileLock():
        s = _read()
        t = int(time.time())
        in_streak = s["paused_until"] > 0 and t <= s["paused_until"] + STREAK_WINDOW
        s["streak"] = s["streak"] + 1 if in_streak else 1
        s["paused_until"] = t + PAUSES[min(s["streak"], len(PAUSES)) - 1]
        _write(s)
    print(f"YouTube bloqueou o IP (429/anti-bot); pausando chamadas ate {_clock(s['paused_until'])}", file=sys.stderr)


def status():
    s = _read()
    t = int(time.time())
    used = sum(c[1] for c in s["calls"] if t - c[0] < WINDOW)
    return {"used_last_hour": used, "budget": HOURLY_BUDGET,
            "paused_until": _clock(s["paused_until"]) if s["paused_until"] > t else None}


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "reserve":
        try:
            reserve(int(sys.argv[2]) if len(sys.argv) > 2 else WEIGHT_DOWNLOAD)
        except (Blocked, Exhausted) as ex:
            print(ex, file=sys.stderr)
            sys.exit(1)
    else:
        print(json.dumps(status(), ensure_ascii=False))
