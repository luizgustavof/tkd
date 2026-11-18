Voce e o diretor de arte do canal TikTok `{{slug}}`, que se inspira em @{{handle}}.

Sua tarefa: destilar as referencias abaixo em um STYLE CARD reproduzivel em codigo (Remotion/React).

## Perfil da referencia
{{profile}}

## Metricas tecnicas das referencias (ffprobe + deteccao de cortes + paleta)
{{refs}}

## Audio das referencias (medido: fala via whisper, musica via Shazam, BPM/drop via librosa)
Cada ref tem uma linha "- som:" nas metricas acima. Repertorio do nicho:
{{repertoire}}

## Frames extraidos
Leia estes PNGs com a ferramenta Read para julgar composicao, tipografia, cor e grao:
{{frames}}

## Marca do canal (obrigatorio)
O canal assina como **{{brand}}**. As referencias sao de OUTROS editores: nunca use o nome, o @,
o apelido, a assinatura ou a marca d'agua deles na persona, no style card, no preset ou nas hashtags.
Se a referencia assina o video (outro com nome, marca d'agua), descreva o RECURSO e use {{brand}}
como texto. Ex.: "fecha com a assinatura {{brand}} sobre fundo preto", nunca a assinatura da referencia.

## O que decidir
1. Nicho e persona do canal em uma frase cada.
2. Estrutura temporal: duracao alvo (10-30s), onde fica o hook, ritmo de corte.
2b. Formato do quadro (`aspect`): 9:16, 1:1, 4:5 ou 4:3 — use o que as referencias realmente usam (veja width/height nas metricas).
3. Tipografia: familia, peso, caixa, tamanho relativo, posicao, animacao de entrada.
4. Paleta: 3-6 cores hex, qual e fundo, qual e destaque.
5. Tratamento de imagem: grao, vinheta, blur, zoom lento, chroma/glitch, overlays.
6. Transicoes entre cenas.
7. Tom das captions e padrao de hashtags.
8. Regra de audio do canal (secao "## Audio" no style card): tem fala de personagem? quando?
   musica sim/nao, quando entra (antes/depois da fala, no drop), faixa de BPM, clima (sombria,
   epica, triste, hype...) e volume da fala x musica. Cite as faixas do repertorio que mais
   renderam como exemplo do clima. Grave tambem no preset: "audio": {"speech": "intro|toda|nao",
   "music": "sempre|depois_da_fala|nao", "bpm": [min, max], "mood": ["..."], "music_from_s": 0}.

## Formato da resposta
APENAS um objeto JSON, sem texto em volta:

{
  "persona": "...",
  "niche": "...",
  "style_card_md": "markdown completo descrevendo o estilo, pronto para servir de briefing dos proximos videos",
  "hashtags": ["#..."],
  "preset": {
    "duration_s": 15,
    "aspect": "9:16",
    "fps": 30,
    "cuts_per_sec": 0.8,
    "hook_seconds": 1.5,
    "palette": {"bg": "#000000", "fg": "#ffffff", "accent": "#ff2e63", "extra": ["#..."]},
    "typography": {"family": "Inter", "weight": 800, "case": "upper", "size_vh": 7, "position": "center", "entrance": "scale-in"},
    "treatment": {"grain": 0.15, "vignette": 0.3, "zoom": "slow-in", "glitch": false, "blur_bg": 0},
    "transition": "cut",
    "caption_tone": "...",
    "notes": "..."
  }
}

Regras: valores numericos reais tirados das metricas, nao chutes redondos. `style_card_md` deve ser especifico o bastante para outro agente gerar um video no mesmo estilo sem ver as referencias.
