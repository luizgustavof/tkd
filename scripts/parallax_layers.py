"""
Separa um frame em camadas para parallax 2.5D: personagem recortado (frente) + fundo preenchido (tras).

uso (venv do rembg: tools/rembg-venv):
  tools/rembg-venv/Scripts/python scripts/parallax_layers.py CLIP.mp4 SEGUNDO OUT_DIR [--model isnet-anime]
gera OUT_DIR/<nome>-fg.png (RGBA), <nome>-bg.png (fundo com o buraco do personagem preenchido) e <nome>-mask.png
"""
import argparse, os, subprocess
import cv2
import numpy as np
from rembg import new_session, remove


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("clip")
    ap.add_argument("t", type=float)
    ap.add_argument("out_dir")
    ap.add_argument("--model", default="isnet-anime")
    a = ap.parse_args()
    os.makedirs(a.out_dir, exist_ok=True)
    name = f"{os.path.splitext(os.path.basename(a.clip))[0]}-{a.t:.2f}"
    frame_p = os.path.join(a.out_dir, f"{name}-frame.png")
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{a.t:.3f}", "-i", a.clip, "-frames:v", "1", frame_p], check=True)
    img = cv2.imread(frame_p)
    rgba = remove(cv2.cvtColor(img, cv2.COLOR_BGR2RGB), session=new_session(a.model))
    alpha = np.asarray(rgba)[:, :, 3]
    fg = np.dstack([img, alpha])
    cv2.imwrite(os.path.join(a.out_dir, f"{name}-fg.png"), fg)
    # fundo: aumenta a mascara (sem halo do contorno) e preenche o buraco
    hole = cv2.dilate((alpha > 24).astype(np.uint8) * 255, np.ones((25, 25), np.uint8))
    small = cv2.resize(img, None, fx=0.5, fy=0.5, interpolation=cv2.INTER_AREA)
    hs = cv2.resize(hole, (small.shape[1], small.shape[0]), interpolation=cv2.INTER_NEAREST)
    bg = cv2.inpaint(small, hs, 12, cv2.INPAINT_TELEA)
    bg = cv2.resize(bg, (img.shape[1], img.shape[0]), interpolation=cv2.INTER_CUBIC)
    bg = np.where(hole[..., None] > 0, bg, img)
    cv2.imwrite(os.path.join(a.out_dir, f"{name}-bg.png"), bg)
    cv2.imwrite(os.path.join(a.out_dir, f"{name}-mask.png"), alpha)
    cover = (alpha > 24).mean()
    print(f"camadas: {name} (personagem cobre {cover:.0%} do quadro) -> {a.out_dir}")


if __name__ == "__main__":
    main()
