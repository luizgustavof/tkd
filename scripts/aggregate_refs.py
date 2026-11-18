"""
Agrega measure.json de varias refs de um canal: o que e recorrente x exclusivo.
uso: python scripts/aggregate_refs.py analysis/<canal> [OUT.json]
"""
import json, sys, os, glob
import numpy as np


def on_beat(cuts, beats, fps, tol=0.07):
    if not beats or not cuts:
        return None
    b = np.array(beats)
    return round(float(np.mean([np.min(np.abs(b - c / fps)) <= tol for c in cuts])), 2)


def summarize(m):
    fps = m["probe"]["fps"]
    sh = m["shots"]
    d = np.array([s["dur_s"] for s in sh])
    rgb = np.array([s["rgb"] for s in sh], dtype=float)
    w = d / d.sum()
    mean_rgb = (rgb * w[:, None]).sum(0)
    cams = [s["camera"] for s in sh if s.get("camera")]
    zin = [c["scale_end"] for c in cams if c["scale_end"] > 1.01]
    zout = [c["scale_end"] for c in cams if c["scale_end"] < 0.99]
    mu = m.get("music") or {}
    first_cut = m["cuts"][0] / fps if m["cuts"] else None
    voice = m.get("voice") or []
    return {
        "duration": m["probe"]["duration"],
        "size": f'{m["probe"]["width"]}x{m["probe"]["height"]}',
        "shots": len(sh),
        "shot_med_s": round(float(np.median(d)), 2),
        "shot_p10_s": round(float(np.quantile(d, .1)), 2),
        "shot_p90_s": round(float(np.quantile(d, .9)), 2),
        "first_shot_s": round(first_cut, 2) if first_cut else None,
        "flashes_per_min": round(len(m["flashes"]) / m["probe"]["duration"] * 60, 1),
        "luma_p50_med": round(float(np.median([s["luma"]["p50"] for s in sh])), 1),
        "black_frac_med": round(float(np.median([s["black_frac"] for s in sh])), 3),
        "clip_frac_med": round(float(np.median([s["clip_frac"] for s in sh])), 3),
        "sat_med": round(float(np.median([s["sat"] for s in sh])), 1),
        "rgb_ratio": [round(float(x / mean_rgb.max()), 3) for x in mean_rgb],
        "zoom_in_frac": round(len(zin) / max(1, len(cams)), 2),
        "zoom_out_frac": round(len(zout) / max(1, len(cams)), 2),
        "zoom_in_med": round(float(np.median(zin)), 3) if zin else None,
        "jitter_y_med": round(float(np.median([c.get("jitter_y", 0) for c in cams])), 1) if cams else None,
        "beat_s": mu.get("beat_s"),
        "cuts_on_beat": on_beat(m["cuts"], mu.get("beats"), fps),
        "voice_lines": len(voice),
        "voice_first_s": voice[0]["start"] if voice else None,
        "voice_sample": [v["text"] for v in voice[:3]],
    }


def main():
    root = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(root, "aggregate.json")
    refs = {}
    for p in sorted(glob.glob(os.path.join(root, "*", "measure.json"))):
        rid = os.path.basename(os.path.dirname(p))
        refs[rid] = summarize(json.load(open(p, encoding="utf-8")))
    keys = [k for k, v in next(iter(refs.values())).items() if isinstance(v, (int, float))]
    stats = {}
    for k in keys:
        vals = [r[k] for r in refs.values() if isinstance(r.get(k), (int, float))]
        if vals:
            stats[k] = {"min": min(vals), "med": round(float(np.median(vals)), 3), "max": max(vals)}
    json.dump({"refs": refs, "stats": stats}, open(out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"{len(refs)} refs -> {out}")
    for k, s in stats.items():
        print(f"{k:16s} {s}")
    for rid, r in refs.items():
        print(rid, {k: r[k] for k in ("duration", "size", "shots", "shot_med_s", "flashes_per_min", "luma_p50_med", "sat_med", "rgb_ratio", "zoom_in_frac", "cuts_on_beat", "voice_lines")})


if __name__ == "__main__":
    main()
