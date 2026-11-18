"""Loudnorm em 2 passadas (mesmo filtro do recreate.rs), sobrescreve o mp4. uso: python scripts/loudnorm.py VIDEO.mp4 [LUFS]"""
import json, os, re, subprocess, sys

src = sys.argv[1]
lufs = float(sys.argv[2]) if len(sys.argv) > 2 else -12.0
af = f"loudnorm=I={lufs}:TP=-1.0:LRA=11"
r = subprocess.run(["ffmpeg", "-hide_banner", "-i", src, "-vn", "-af", af + ":print_format=json", "-f", "null", "-"],
                   capture_output=True, text=True, encoding="utf-8", errors="replace")
m = json.loads(re.findall(r"\{[^{}]*\}", r.stderr)[-1])
af += (f":measured_I={m['input_i']}:measured_TP={m['input_tp']}:measured_LRA={m['input_lra']}"
       f":measured_thresh={m['input_thresh']}:offset={m['target_offset']}:linear=true")
tmp = src[:-4] + ".norm.mp4"
subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", src, "-c:v", "copy", "-af", af, "-ar", "48000",
                "-c:a", "aac", "-b:a", "320k", tmp], check=True)
os.replace(tmp, src)
print(f"loudnorm: {src} {m['input_i']} LUFS -> {lufs}")
