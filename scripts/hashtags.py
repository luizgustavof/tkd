"""
Separa as hashtags da legenda de um post em 3 grupos:

  genericas : alcance/algoritmo, nao dizem nada do video (#fyp, #foryou, #viral, #edit, #capcut ...)
  nicho     : comunidade/formato da edit (#animeedit, #movieedit, #lyricsedit, #aesthetic, #nichetok, #larp ...)
  contexto  : do proprio video — obra, personagem, ator, assunto (#jujutsukaisen, #toji, #limitless, #gta6 ...)

Variantes '<algo>edit(s)' de contexto viram o radical (#tojiedit -> toji) e entram em contexto.
uso como modulo: split_hashtags(texto) -> {"genericas": [...], "nicho": [...], "contexto": [...], "legenda": texto sem #}
"""
import re

GENERIC = {
    "fyp", "foryou", "foryoupage", "fy", "fypage", "viral", "trending", "trend", "xyzbca", "capcut", "tiktok", "explore",
    "parati", "paravoce", "pravoce", "edit", "edits", "myedit", "myedits", "editor", "editing", "aftereffects", "ae", "alightmotion",
    "fypシ", "fypage", "blowthisup", "blowthisupforme", "goviral", "fyppage", "4u", "4you", "foru", "recommended", "repost",
    "video", "videos", "edite", "ediçao", "edição", "editz", "edt", "vfx", "4k", "hd", "qualitystuff",
}
GENERIC_RE = re.compile(r"^(f+y+p+.*|for+you+.*|fy+|viral.*|xy[zb].*|trend.*|blowthis.*|tiktok.*)$")
NICHE = {
    "anime", "animeedit", "animeedits", "animefyp", "animes", "manga", "mangaedit", "otaku", "weeb", "amv", "movie", "movies",
    "movieedit", "movieedits", "film", "films", "filmes", "filmedit", "cinema", "cinematic", "series", "tvshow", "musicedit",
    "lyricsedit", "lyrics", "lyricvideo", "aesthetic", "darkaesthetic", "aestheticedit", "nichetok", "targetaudience", "larp",
    "larping", "peak", "sigma", "motivation", "gaming", "gamer", "games", "indiegames", "horror", "rocknroll", "numetal", "rock",
    "phonk", "slowed", "sad", "sadedit", "edits4u", "founder", "startups", "investor", "ai", "study", "luxury", "money",
    "cartoon", "cartoonedit", "superhero", "marvel", "dc", "4kedit", "realedit", "hopecore", "corecore", "citypop",
    "brutalism", "surrealism", "core", "vibes", "edgy", "dark",
}


def _norm(t):
    return t.lower().strip(".,!?:;)(\"'").replace("’", "'")


def split_hashtags(text, creator=None):
    tags = []
    for t in re.findall(r"#([^\s#]+)", text or ""):
        t = _norm(t)
        if t and t not in tags:
            tags.append(t)
    out = {"genericas": [], "nicho": [], "contexto": []}
    for t in tags:
        if creator and re.sub(r"\W", "", t) == re.sub(r"\W", "", creator.lower()) or re.match(r"^[a-z]+\d{2,}$", t) and creator and t.startswith(creator.lower()[:4]):
            out["genericas"].append(t)  # @ do proprio criador
        elif t in GENERIC or GENERIC_RE.match(t):
            out["genericas"].append(t)
        elif t in NICHE:
            out["nicho"].append(t)
        else:
            root = re.sub(r"(edits?|editz|fyp)$", "", t)
            out["contexto"].append(root if len(root) >= 3 and root not in GENERIC and root not in NICHE else t)
    seen = set()
    out["contexto"] = [t for t in out["contexto"] if not (t in seen or seen.add(t))]
    out["legenda"] = re.sub(r"\s+", " ", re.sub(r"#[^\s#]+", "", text or "")).strip(" |-—")
    return out


if __name__ == "__main__":
    import glob, json, sys
    for p in sorted(glob.glob("dataset/raw/*.info.json"))[: int(sys.argv[1]) if len(sys.argv) > 1 else 8]:
        d = json.load(open(p, encoding="utf-8"))
        print(d["id"], json.dumps(split_hashtags(d.get("description"), d.get("uploader")), ensure_ascii=False))
