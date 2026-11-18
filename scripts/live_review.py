"""
Pagina local para revisar os clipes de live (nada sai do PC).

uso: python scripts/live_review.py out/live [--open]
Gera out/live/index.html com cada clipe (9:16 e original), nota, pesos dos sinais, transcricao e chat do trecho.
"""
import glob, html, json, os, sys


def main():
    root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "out/live")
    cards = []
    for j in sorted(glob.glob(os.path.join(root, "*", "clips", "clip_*.json")), key=os.path.getmtime):
        try:
            d = json.load(open(j, encoding="utf-8"))
        except Exception:
            continue
        rel = os.path.relpath(os.path.dirname(j), root).replace("\\", "/")
        f = d.get("files", {})
        vid = f"{rel}/{f.get('vertical') or f.get('raw')}"
        raw = f"{rel}/{f.get('raw')}"
        bd = d.get("breakdown", {})
        bars = "".join(
            f'<div class="b"><span>{html.escape(k)}</span><i style="width:{max(0, min(100, v * 40)):.0f}%"></i>'
            f'<em>{v:+.2f}</em></div>' for k, v in sorted(bd.items(), key=lambda x: -x[1]))
        chat = "".join(f"<li>{html.escape(m)}</li>" for m in d.get("chat", [])[:40])
        cards.append(f"""
<article>
  <video src="{html.escape(vid)}" controls preload="metadata" playsinline></video>
  <div class="info">
    <h2>{html.escape(d.get('platform', ''))} · {html.escape(d.get('channel', ''))} · {os.path.basename(j)[:-5]}</h2>
    <p class="meta">nota <b>{d.get('score', 0):+.2f}</b> · {d.get('duration', 0):.0f}s ·
       {html.escape(d.get('created', ''))} · <a href="{html.escape(raw)}">original 16:9</a></p>
    <div class="bars">{bars}</div>
    <h3>Fala (whisper)</h3><p class="tr">{html.escape(d.get('transcript') or '—')}</p>
    <h3>Chat no trecho ({len(d.get('chat', []))})</h3><ul class="chat">{chat or '<li>—</li>'}</ul>
  </div>
</article>""")
    page = f"""<!doctype html><html lang="pt-BR"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Clipes de live</title>
<style>
:root{{--bg:#0f1115;--card:#181b22;--fg:#e8e8ea;--mut:#9aa0aa;--acc:#ff4d6d;--bar:#3a7bfd}}
body{{margin:0;background:var(--bg);color:var(--fg);font:15px/1.45 system-ui,Segoe UI,sans-serif}}
header{{padding:20px 16px 4px}} h1{{margin:0;font-size:22px}} header p{{color:var(--mut);margin:4px 0 0}}
main{{display:grid;gap:16px;padding:16px;grid-template-columns:repeat(auto-fill,minmax(min(100%,560px),1fr))}}
article{{background:var(--card);border-radius:12px;padding:12px;display:flex;gap:14px}}
video{{width:200px;aspect-ratio:9/16;background:#000;border-radius:8px;flex:none}}
.info{{min-width:0;flex:1}} h2{{font-size:15px;margin:0 0 4px}} h3{{font-size:12px;color:var(--mut);margin:10px 0 4px;text-transform:uppercase}}
.meta{{color:var(--mut);margin:0}} .meta b{{color:var(--acc)}} a{{color:#7aa7ff}}
.b{{display:grid;grid-template-columns:70px 1fr 48px;align-items:center;gap:6px;font-size:12px;color:var(--mut)}}
.b i{{display:block;height:6px;background:var(--bar);border-radius:3px}} .b em{{font-style:normal;text-align:right}}
.tr{{margin:0;font-size:13px}} .chat{{margin:0;padding-left:16px;max-height:140px;overflow:auto;font-size:12px;color:var(--mut)}}
@media (max-width:560px){{article{{flex-direction:column}} video{{width:100%}}}}
</style></head><body>
<header><h1>Clipes de live</h1><p>{len(cards)} clipes · arquivos locais em {html.escape(root)}</p></header>
<main>{''.join(cards) or '<p>nenhum clipe ainda</p>'}</main></body></html>"""
    out = os.path.join(root, "index.html")
    open(out, "w", encoding="utf-8").write(page)
    print(out)
    if "--open" in sys.argv:
        os.startfile(out)


if __name__ == "__main__":
    main()
