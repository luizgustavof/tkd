"""
Leitura publica do TikTok (sem conta) com o Scrapling: substitui browser/scrape_channel.mjs, download.mjs e
comments.mjs. Mesmo contrato de entrada/saida dos .mjs (ver scripts/web.py).

uso:
  python scripts/tiktok_web.py channel  --json '{"handle":"perfil.exemplo","limit":20}'
  python scripts/tiktok_web.py download --json '{"handle":"perfil.exemplo","videos":[{"id","url"}],"outDir":"..."}'
  python scripts/tiktok_web.py comments --json '{"videos":[{"id","url"}],"outDir":"dataset/raw","max":30}'
                                    (abre so o perfil do autor e le /api/comment/list/ de dentro da pagina)

O que muda em relacao aos .mjs:
- navegador anonimo (perfil .profiles/_scrapling), nao o perfil logado do canal: o scout nao disputa a trava do
  Edge logado (login/upload) e nao arrisca a conta;
- dados vem das respostas da propria pagina (/api/post/item_list/, /api/comment/list/) e do JSON embutido
  (__UNIVERSAL_DATA_FOR_REHYDRATION__), nao de seletores de DOM;
- toda pagina passa pelo ritmo de web.tiktok_pace; muro detectado pausa o TikTok para todos os processos;
- download: o tkd tenta yt-dlp antes; este modo e o plano B (playAddr da API, baixado com os cookies da pagina).
"""
import json, os, random, re, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from web import args, emit, evt, log, human, browser_session, tiktok_pace, tiktok_wall, Walled  # noqa: E402

GRID_SEL = 'a[href*="/video/"]'
# botao que o TikTok mostra junto de "Algo deu errado" quando a grade vem vazia
REFRESH_LABEL = re.compile(r"^\s*(atualizar|tentar novamente|refresh|try again|retry)\s*$", re.I)


def click_refresh(page):
    """Clicar em Atualizar refaz so o pedido da lista, sem recarregar a pagina (mais humano, menos peso no IP).
    True se achou o botao e clicou."""
    try:
        for b in page.get_by_role("button", name=REFRESH_LABEL).all()[:3]:
            if b.is_visible():
                b.scroll_into_view_if_needed(timeout=2000)
                human(page, 600, 1500)
                b.click(timeout=3000)
                log("grade vazia: cliquei em atualizar")
                return True
    except Exception:
        pass
    return False


def wait_for_content(page, sel, tries=2, kind="profile"):
    """Espera o conteudo real. Grade vazia: primeiro clica em Atualizar (ate 2x, com pausa), depois recarrega
    (cada reload passa pelo ritmo). Nao achou depois de `tries` = muro: registra a pausa para ninguem martelar o IP."""
    clicks = 0
    for i in range(tries):
        try:
            page.wait_for_selector(sel, timeout=20000)
            return True
        except Exception:
            try:
                body = page.evaluate("document.body.innerText.slice(0,300)")
            except Exception:
                body = ""
            log(f"tentativa {i + 1}: sem '{sel}'. body={re.sub(r'\s+', ' ', body).strip()!r}")
            while clicks < 2:
                human(page, 3000, 6000)
                if not click_refresh(page):
                    break
                clicks += 1
                try:
                    page.wait_for_selector(sel, timeout=15000)
                    return True
                except Exception:
                    pass
            if i + 1 == tries:
                break
            try:
                tiktok_pace(kind)
                page.reload(wait_until="domcontentloaded")
            except Walled:
                break
            except Exception:
                pass
    tiktok_wall()
    return False


def scroll(page, steps):
    for _ in range(steps):
        page.mouse.wheel(0, 600 + random.randint(0, 700))
        human(page, 500, 1400)


def rehydration(page):
    try:
        m = re.search(r'id="__UNIVERSAL_DATA_FOR_REHYDRATION__"[^>]*>(.*?)</script>', page.content(), re.S)
        return json.loads(m.group(1)).get("__DEFAULT_SCOPE__", {}) if m else {}
    except Exception:
        return {}


def api_items(sess, after_id=0):
    """Itens das respostas /api/post/item_list/ gravadas pela sessao (corpo vazio = descartado)."""
    out = {}
    for r in sess.network.search(url_pattern=r"/api/post/item_list/", after_id=after_id, limit=None):
        try:
            for it in (r.json().get("itemList") or []) if r.body else []:
                out[str(it.get("id"))] = it
        except Exception:
            pass
    return out


def fmt_n(n):
    """1234567 -> "1.2M", como a pagina mostra."""
    if not isinstance(n, (int, float)):
        return str(n or "")
    for lim, suf in ((1e9, "B"), (1e6, "M"), (1e3, "K")):
        if n >= lim:
            return f"{n / lim:.1f}".rstrip("0").rstrip(".") + suf
    return str(int(n))


def item_meta(it):
    stats = it.get("stats") or {}
    return {
        "caption": it.get("desc") or "",
        "music": (it.get("music") or {}).get("title") or "",
        "likes": fmt_n(stats.get("diggCount")),
        "comments": fmt_n(stats.get("commentCount")),
        "shares": fmt_n(stats.get("shareCount")),
        "hashtags": [c.get("title") for c in it.get("challenges") or [] if c.get("title")][:15],
    }


def read_grid(page):
    return page.evaluate("""() => {
      const out = [], seen = new Set();
      for (const a of document.querySelectorAll('a[href*="/video/"]')) {
        const id = a.href.split("/video/")[1]?.split(/[?#]/)[0];
        if (!id || seen.has(id)) continue;
        seen.add(id);
        const card = a.closest("div");
        out.push({ id, url: a.href, caption: a.querySelector("img")?.alt ?? "",
          views: card?.querySelector('[data-e2e="video-views"]')?.textContent?.trim() ?? "",
          cover: a.querySelector("img")?.src ?? "" });
      }
      return out;
    }""")


# ------------------------------------------------------------------ channel

def channel(p):
    handle, limit = (p.get("handle") or "").lstrip("@"), int(p.get("limit", 20))
    if not handle:
        return {"ok": False, "error": "handle obrigatorio"}
    url = f"https://www.tiktok.com/@{handle}"
    res = {"ok": False, "error": "pagina nao abriu", "url": url}
    with browser_session(record=True) as s:
        def act(page):
            evt(action=f"Olhando o perfil @{handle}", url=url)
            human(page, 1500, 3000)
            if not wait_for_content(page, GRID_SEL):
                res.update(error="muro anti-bot ou perfil sem videos", walled=True)
                return
            scope = rehydration(page)
            info = (scope.get("webapp.user-detail") or {}).get("userInfo") or {}
            user, stats = info.get("user") or {}, info.get("stats") or {}
            evt(action=f"Rolando a grade de @{handle}")
            scroll(page, -(-limit // 8) + 2)
            api = api_items(s)
            text = lambda sel: page.evaluate(f"document.querySelector('{sel}')?.textContent?.trim() ?? ''")
            account = {
                "nickname": user.get("nickname") or text('[data-e2e="user-subtitle"]'),
                "signature": user.get("signature") if "signature" in user else text('[data-e2e="user-bio"]'),
                "followers": fmt_n(stats["followerCount"]) if "followerCount" in stats
                else text('[data-e2e="followers-count"]'),
                "likes": fmt_n(stats["heartCount"]) if "heartCount" in stats else text('[data-e2e="likes-count"]'),
            }
            photos = page.evaluate("new Set([...document.querySelectorAll('a[href*=\"/photo/\"]')].map(a=>a.href)).size")
            videos = read_grid(page)
            for v in videos:  # a grade mostra views abreviado; a API tem o numero certo e a legenda inteira
                it = api.get(v["id"])
                if it:
                    v["views"] = fmt_n((it.get("stats") or {}).get("playCount")) or v["views"]
                    v["caption"] = it.get("desc") or v["caption"]
            res.clear()
            res.update(ok=True, handle=handle, account=account, photos=photos, videos=videos[:limit])

        tiktok_pace("profile")
        s.fetch(url, page_action=act, wait=0)
    return res


# ------------------------------------------------------------------ download

def download(p):
    handle, out_dir, videos = (p.get("handle") or "").lstrip("@"), p.get("outDir"), p.get("videos") or []
    if not out_dir or not handle:
        return {"ok": False, "error": "outDir e handle sao obrigatorios"}
    os.makedirs(out_dir, exist_ok=True)
    grid_url = f"https://www.tiktok.com/@{handle}"
    done, failed = [], []
    res = {"ok": False, "error": "pagina nao abriu"}

    with browser_session(record=True) as s:
        def act(page):
            human(page, 1500, 3000)
            if not wait_for_content(page, GRID_SEL):
                res.update(error="muro anti-bot na grade do perfil", walled=True)
                return
            wanted = {str(v["id"]) for v in videos}
            # a API so devolve as paginas que a rolagem carrega; rola ate achar todos (ou parar de crescer)
            api, prev = api_items(s), -1
            while not wanted <= set(api) and len(api) != prev:
                prev = len(api)
                scroll(page, 3)
                api = api_items(s)
            for k, v in enumerate(videos, 1):
                vid = str(v["id"])
                evt(action=f"Baixando vídeo {k} de {len(videos)}", progress=(k - 1) / len(videos) * 100, url=v.get("url"))
                try:
                    it = api.get(vid)
                    src, meta = None, {}
                    if it:
                        video = it.get("video") or {}
                        src, meta = video.get("playAddr") or video.get("downloadAddr"), item_meta(it)
                    if not src:
                        src = click_and_capture(page, vid, grid_url)
                    if not src:
                        failed.append({"id": vid, "error": "player nao expos a midia"})
                        continue
                    r = page.context.request.get(src, headers={"referer": "https://www.tiktok.com/", "range": "bytes=0-"},
                                                 timeout=120000)
                    if not r.ok and r.status != 206:
                        failed.append({"id": vid, "error": f"http {r.status}"})
                        continue
                    path = os.path.join(out_dir, f"{vid}.mp4")
                    with open(path, "wb") as fh:
                        fh.write(r.body())
                    mb = os.path.getsize(path) / 1e6
                    if mb < 0.05:
                        os.remove(path)
                        failed.append({"id": vid, "error": "arquivo vazio"})
                        continue
                    done.append({"id": vid, "url": v.get("url") or f"{grid_url}/video/{vid}", "file": path, **meta})
                    log(f"baixado {vid} ({mb:.1f} MB)")
                    human(page, 1500, 3500)
                except Exception as e:
                    failed.append({"id": vid, "error": str(e).split("\n")[0]})
            res.clear()
            res.update(ok=True, downloaded=done, failed=failed)

        tiktok_pace("profile")
        s.fetch(grid_url, page_action=act, wait=0)
    return res


def click_and_capture(page, vid, grid_url):
    """Item fora da API (1a pagina veio no HTML): mesma tatica do download.mjs, clica no thumbnail e pega a URL do
    CDN na resposta de rede. Navegar direto para /video/<id> costuma ser bloqueado."""
    target = page.locator(f'a[href*="/video/{vid}"]').first
    if target.count() == 0:
        page.mouse.wheel(0, 1200)
        human(page, 800, 1600)
    if target.count() == 0:
        return None
    try:
        with page.expect_response(lambda r: re.search(r"/video/tos/|\.mp4", r.url) and r.status < 400,
                                  timeout=45000) as info:
            target.scroll_into_view_if_needed()
            human(page, 400, 1200)
            target.click()
        src = info.value.url
    except Exception:
        src = None
    human(page, 1200, 2500)
    try:
        page.keyboard.press("Escape")
        page.wait_for_timeout(800)
        if "/video/" in page.url or f"@{grid_url.rsplit('@', 1)[1]}" not in page.url:
            tiktok_pace("profile")
            page.goto(grid_url, wait_until="domcontentloaded")
            wait_for_content(page, GRID_SEL)
    except Exception:
        pass
    return src


# ------------------------------------------------------------------ comments

def norm(t):
    import unicodedata
    t = unicodedata.normalize("NFKC", (t or "").lower())
    return re.sub(r"[!.?,;:]+$", "", re.sub(r"\s+", " ", t)).strip()


def collapse(got):
    """Textos iguais de pessoas diferentes viram um so, com `dupes` e likes/replies somados."""
    by = {}
    for c in got.values():
        k = norm(c["text"])
        if not k:
            continue
        if k in by:
            by[k]["dupes"] += 1
            by[k]["likes"] += c["likes"]
            by[k]["replies"] += c["replies"]
        else:
            by[k] = {**c, "dupes": 1}
    return sorted(by.values(), key=lambda c: -c["likes"])


# Abrir /video/<id> direto como visitante e bloqueado (ERR_HTTP_RESPONSE_CODE_FAILURE). O que funciona: abrir UMA
# pagina do TikTok (perfil do autor) e chamar /api/comment/list/ com fetch de dentro dela; o SDK da propria pagina
# assina a requisicao. Um navegador, uma pagina, N videos: bem menos peso no IP que abrir cada video.
COMMENT_PAGE_JS = """async ([id, cursor]) => {
  const r = await fetch(`/api/comment/list/?aid=1988&aweme_id=${id}&count=20&cursor=${cursor}`, {credentials: 'include'});
  const t = await r.text();
  try { return { status: r.status, len: t.length, json: JSON.parse(t) }; }
  catch { return { status: r.status, len: t.length, json: null }; }
}"""


def comment_row(c):
    return {
        "cid": c.get("cid"),
        "text": c.get("text") or (c.get("share_info") or {}).get("desc") or "",
        "likes": c.get("digg_count") or 0,
        "replies": c.get("reply_comment_total") or 0,
        "create_time": c.get("create_time"),
        "lang": c.get("comment_language") or "",
    }


def read_comments(page, vid, max_n, deadline):
    """Pagina /api/comment/list/ ate max_n, has_more=0, cursor repetido ou prazo. Corpo vazio = muro (levanta)."""
    t0, got, cursors, cursor, more, responses, stop = time.time(), {}, set(), 0, True, 0, ""
    while True:
        r = page.evaluate(COMMENT_PAGE_JS, [vid, cursor])
        if r["status"] == 200 and r["len"] == 0:
            tiktok_wall()
            raise Walled("API de comentarios respondeu vazio (muro anti-bot)")
        j = r["json"] or {}
        responses += 1
        for c in j.get("comments") or []:
            got.setdefault(c.get("cid"), comment_row(c))
        if j.get("has_more") in (0, False, None):
            more, stop = False, "fim da lista"
        nxt = j.get("cursor")
        if more and (nxt is None or nxt in cursors):
            stop = "pagina repetida"
        elif more and len(got) >= max_n:
            stop = "max"
        elif more and time.time() - t0 >= deadline:
            stop = "prazo"
        if stop:
            return got, stop, responses, ("" if got else f"http {r['status']}, {r['len']} bytes")
        cursors.add(nxt)
        cursor = nxt
        human(page, 1200, 2500)


def comments(p):
    videos, out_dir = p.get("videos") or [], p.get("outDir", "dataset/raw")
    max_n, deadline = int(p.get("max", 30)), float(p.get("deadlineMs", 45000)) / 1000
    os.makedirs(out_dir, exist_ok=True)
    result = []
    pending = [v for v in videos if not os.path.exists(os.path.join(out_dir, f"{v['id']}.comments.json"))]
    result += [{"id": v["id"], "ok": True, "cached": True} for v in videos if v not in pending]
    if not pending:
        return {"ok": True, "result": result}

    m = re.search(r"tiktok\.com/@([\w.\-]+)", pending[0].get("url") or "")
    base = f"https://www.tiktok.com/@{m.group(1)}" if m else "https://www.tiktok.com/explore"
    with browser_session() as s:
        def act(page):
            human(page, 2500, 4000)
            for k, v in enumerate(pending, 1):
                evt(action=f"Lendo comentários do vídeo {k} de {len(pending)}", progress=(k - 1) / len(pending) * 100,
                    url=v["url"])
                t0 = time.time()
                try:
                    tiktok_pace("video")
                    got, stop, responses, body = read_comments(page, str(v["id"]), max_n, deadline)
                except Walled as e:
                    log(str(e))
                    result.append({"id": v["id"], "ok": False, "stop": "muro", "body": str(e)})
                    return
                except Exception as e:
                    got, stop, responses, body = {}, "erro", 0, str(e)[:160]
                arr = collapse(got)
                if arr:
                    with open(os.path.join(out_dir, f"{v['id']}.comments.json"), "w", encoding="utf-8") as fh:
                        json.dump(arr, fh, ensure_ascii=False, indent=1)
                dt = time.time() - t0
                log(f"{v['id']}: {len(arr)} comentarios ({len(got)} brutos, {responses} respostas, parou: {stop}, "
                    f"{dt:.0f}s) {body}")
                result.append({"id": v["id"], "ok": bool(arr), "n": len(arr), "raw": len(got), "secs": round(dt),
                               "stop": stop, "body": body})

        tiktok_pace("profile")
        s.fetch(base, page_action=act, wait=0)
    return {"ok": True, "result": result}


MODES = {"channel": channel, "download": download, "comments": comments}

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode not in MODES:
        emit({"ok": False, "error": f"modo invalido: {mode!r} (use {', '.join(MODES)})"})
        sys.exit(0)
    try:
        emit(MODES[mode](args()))
    except Walled as e:
        emit({"ok": False, "error": str(e), "walled": True})
    except Exception as e:
        emit({"ok": False, "error": f"{type(e).__name__}: {e}"})
