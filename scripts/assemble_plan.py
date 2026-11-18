"""
Junta o plano do agente (shots/estilo) com a voz (voice.json de voice_lines.py) e gera as props
finais da composicao Remotion `recreate` (tipo RecreatePlan em video/src/recreate/types.ts).

uso: python scripts/assemble_plan.py PLAN.json OUT_PROPS.json [--voice VOICE.json --voice-wav REL]
  PLAN.json   : saida do agente + autograde (shots com grade, captionStyle, title, frameFx, blooms,
                watermark, look, audio.music/musicStart, durationInFrames, width, height, fps)
  --voice     : voice.json; captions kind=caption viram CaptionTrack, kind=title viram TitleStack
  --voice-wav : caminho do voice.wav relativo a video/public (vai para audio.voice)
"""
import json, argparse


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("plan")
    ap.add_argument("out")
    ap.add_argument("--voice")
    ap.add_argument("--voice-wav")
    a = ap.parse_args()
    p = json.load(open(a.plan, encoding="utf-8"))
    fps = p.get("fps", 30)
    total = p.get("durationInFrames") or max(s["to"] for s in p["shots"])
    props = {
        "id": p.get("id", "recreate"),
        "fps": fps,
        "width": p.get("width", 1080),
        "height": p.get("height", 1080),
        "durationInFrames": total,
        "shots": p["shots"],
        "captionStyle": p.get("captionStyle", {"sizePx": 44}),
        "title": None,
        "captions": p.get("captions", []),
        "frameFx": {str(k): v for k, v in (p.get("frameFx") or {}).items()},
        "blooms": p.get("blooms", []),
        "watermark": p.get("watermark"),
        "look": p.get("look", {}),
        "endCard": p.get("endCard"),
        "voiceClips": p.get("voiceClips", []),
        "motion": p.get("motion", []),
        "audio": dict(p.get("audio", {})),
    }
    title_style = p.get("titleStyle") or {}
    if a.voice:
        v = json.load(open(a.voice, encoding="utf-8"))
        # ordem de tempo: clip_voice gera as falas dos planos antes das dos voiceClips, e o "end" de cada legenda
        # e cortado no inicio da proxima da lista (Toji 18/11: legenda do frame 526 ganhou end 6 e sumiu)
        first = lambda c: c["steps"][0]["f"] if c.get("steps") else 0
        caps = sorted((c for c in v.get("captions", []) if c.get("kind", "caption") == "caption"), key=first)
        props["captions"] = [
            {"seed": c.get("id", f"c{i}"), "steps": c["steps"],
             "end": min(c["end"], caps[i + 1]["steps"][0]["f"]) if i + 1 < len(caps) else c["end"]}
            for i, c in enumerate(caps)
        ]
        titles = sorted((c for c in v.get("captions", []) if c.get("kind") == "title" and c.get("steps")), key=first)
        if titles:
            # um TitleStack so: entre um titulo e o proximo entra um passo vazio (esconde). Antes os passos iam
            # fora de ordem com o "end" do ultimo e o 1o titulo ficava na tela ate o fim do ultimo (16 s no Toji)
            steps = []
            for i, c in enumerate(titles):
                steps += c["steps"]
                if i + 1 < len(titles) and c["end"] < first(titles[i + 1]):
                    steps.append({"f": c["end"], "text": ""})
            props["title"] = {**title_style, "steps": steps, "end": titles[-1]["end"]}
    if a.voice_wav:
        props["audio"]["voice"] = a.voice_wav
    for s in props["shots"]:
        s["to"] = min(s["to"], total)
    json.dump(props, open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"props: {len(props['shots'])} planos, {len(props['captions'])} legendas, "
          f"title={'sim' if props['title'] else 'nao'}, {total} frames -> {a.out}")


if __name__ == "__main__":
    main()
