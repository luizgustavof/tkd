#!/usr/bin/env bash
# Validacao visual ref x recriacao.
# uso: scripts/compare.sh REF.mp4 REMAKE.mp4 OUT_DIR [DUR_S] [TIMES...]
#  - corta a ref para DUR_S (padrao: duracao do remake)
#  - sbs.mp4: lado a lado com audio dos dois (ref esquerda)
#  - pairs.jpg: frames pareados nos TIMES (padrao: a cada 0.5s)
#  - sheet_ref.jpg / sheet_remake.jpg: contact sheets 4 fps
#  - captions_ref.png / captions_remake.png: faixa da legenda frame a frame (0-6s)
#  - rea-video compare (compare.json + compare_worst.jpg)
set -euo pipefail
REF="$1"; REMAKE="$2"; OUT="$3"; shift 3
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REA="$ROOT/tools/rea-video/.venv/Scripts/rea-video"
FONT="C\\:/Windows/Fonts/arial.ttf"
mkdir -p "$OUT"
DUR="${1:-$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$REMAKE")}"; [ $# -gt 0 ] && shift
TIMES=("$@"); if [ ${#TIMES[@]} -eq 0 ]; then TIMES=($(python -c "print(' '.join(str(round(x*0.5,2)) for x in range(int(float('$DUR')*2))))")); fi

ffmpeg -v error -y -t "$DUR" -i "$REF" -c:v libx264 -crf 16 -c:a aac "$OUT/ref_cut.mp4"

# lado a lado com rotulo e numero do frame (mantem o aspecto; audio dos dois se ambos tiverem)
has_a() { [ -n "$(ffprobe -v error -select_streams a -show_entries stream=index -of csv=p=0 "$1")" ]; }
if has_a "$OUT/ref_cut.mp4" && has_a "$REMAKE"; then
  AU=";[0:a][1:a]amerge=inputs=2,pan=stereo|c0<c0+c1|c1<c2+c3[au]"; AMAP=(-map "[au]" -c:a aac)
elif has_a "$OUT/ref_cut.mp4"; then AU=""; AMAP=(-map 0:a -c:a aac)
else AU=""; AMAP=(); fi
ffmpeg -v error -y -i "$OUT/ref_cut.mp4" -i "$REMAKE" -filter_complex  "[0:v]scale=540:-2,drawtext=fontfile='$FONT':text='REF %{n}':x=8:y=8:fontsize=20:fontcolor=yellow:box=1:boxcolor=black[a];  [1:v]scale=540:-2,drawtext=fontfile='$FONT':text='NEW %{n}':x=8:y=8:fontsize=20:fontcolor=yellow:box=1:boxcolor=black[b];  [b][a]scale2ref=w=iw:h=ih[b2][a2];[a2][b2]hstack[v]$AU"  -map "[v]" "${AMAP[@]}" -t "$DUR" -c:v libx264 -crf 20 "$OUT/sbs.mp4"

# frames pareados
rows=()
for t in "${TIMES[@]}"; do
  ffmpeg -v error -y -ss "$t" -i "$OUT/sbs.mp4" -frames:v 1 "$OUT/pair_$t.png"
  rows+=("$OUT/pair_$t.png")
done
n=${#rows[@]}; args=(); fc=""
for i in "${!rows[@]}"; do args+=(-i "${rows[$i]}"); fc+="[$i]"; done
cols=2; per=$(( (n + cols - 1) / cols ))
# empilha em 2 colunas
python - "$OUT" "$cols" "${rows[@]}" <<'EOF'
import sys, subprocess
out, cols, files = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
per = -(-len(files)//cols)
inputs=[]; [inputs.extend(["-i",f]) for f in files]
fc=""; colnames=[]
for c in range(cols):
    chunk=list(range(c*per, min((c+1)*per, len(files))))
    if not chunk: continue
    while len(chunk)<per: chunk.append(chunk[-1])
    fc+="".join(f"[{k}]" for k in chunk)+f"vstack=inputs={per}[c{c}];"
    colnames.append(f"[c{c}]")
fc+="".join(colnames)+f"hstack=inputs={len(colnames)},scale=iw/2:-2[o]"
subprocess.run(["ffmpeg","-v","error","-y",*inputs,"-filter_complex",fc,"-map","[o]",f"{out}/pairs.jpg"],check=True)
EOF
rm -f "$OUT"/pair_*.png

for side in ref_cut:sheet_ref "$REMAKE":sheet_remake; do
  src="${side%%:*}"; name="${side##*:}"; [ "$src" = ref_cut ] && src="$OUT/ref_cut.mp4"
  ffmpeg -v error -y -i "$src" -vf "fps=4,scale=240:240,tile=8x6" -frames:v 1 "$OUT/$name.jpg"
  cap="${name/sheet/captions}"
  ffmpeg -v error -y -t 6 -i "$src" -vf "crop=900:70:90:490,scale=450:35,drawtext=fontfile='$FONT':text='%{n}':x=2:y=2:fontsize=14:fontcolor=yellow,tile=5x36" -frames:v 1 "$OUT/$cap.png"
done

PYTHONUTF8=1 PYTHONIOENCODING=utf-8 "$REA" compare "$OUT/ref_cut.mp4" "$REMAKE" -o "$OUT/rea" --every 0.25 >/dev/null 2>&1 || echo "rea-video compare falhou (ver $OUT/rea)"
echo "ok: $OUT"
