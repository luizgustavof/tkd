# QUALITY_RULES — vale para todo vídeo de todo canal

Aplicadas automaticamente por `tkd footage`, `tkd recreate` e `tkd render`; conferidas por `scripts/qa.py`
(nota 0–100, `erro` = não publicar). O agente de recriação recebe este arquivo no prompt.

## Footage
1. **Fonte ≥ 720p, ideal ≥ 1080p.** `tkd footage` baixa com o client `default` do yt-dlp (o `android` só libera 360p)
   e descarta fonte < 720p. Clipe antigo de fonte baixa: `scripts/requalify_clips.py <canal>`.
2. Recorte em H.264 CRF 16 (preset medium), áudio AAC 160k, 30 fps, já no aspecto do canal.
3. Proibido em plano visível > 0.2 s: legenda queimada, logo/marca de terceiros, call-to-action ("subscribe"),
   tela preta, menu. Clipe assim com fala boa fica como **só áudio** (`so-audio`).
4. Macrobloco (`block` > 1.5): evitar em plano > 0.5 s.
5. Zoom digital total (câmera × overscan) ≤ 1.25× em fonte < 1440p.
6. **Um personagem por edit** (`--character`): todo plano e toda fala dele.

## Imagem
7. Nenhuma sequência de 4+ frames pretos não intencional (corte interno da fonte / `startAt` fora do clipe).
8. Grade por plano calibrado pela ref (`autograde.py`); sharpen leve (0.3) sempre, grão por cima (não antes).
9. Legenda legível: ≥ 0.2 s com a frase completa na tela; contorno/sombra sobre fundo claro.

## Áudio
10. Voz = fala real do personagem (`--voice clip`); TTS só sem alternativa.
11. Loudness final −12 LUFS ±1.5 (edits de hype; `TKD_LUFS` por canal), true peak ≤ −0.5 dBFS; música abaixa sob a fala (duck 0.35).

## Render
12. Remotion: `--crf=16 --x264-preset=slow --jpeg-quality=95 --audio-bitrate=320k`, 1080 de largura.
13. Bitrate de vídeo ≥ 6 Mbps (o TikTok recomprime; começar alto preserva grão e preto).

## Movimento (envolvimento)
14. Nenhum plano longo (≥ 1 s) totalmente parado sem intenção: push-in lento 1.00→1.04/1.05 ou respiração (jitter).
15. Canais de alta energia: punch no corte rápido, shake + RGB split nos cortes em onset forte, clarão + punch
    no gatilho do drop, pulso na batida dentro de plano longo, legenda com pop. Canais calmos: só push-in.
    Preset em `scripts/automotion.py` (`motionPreset`: high | medium | low | none).
