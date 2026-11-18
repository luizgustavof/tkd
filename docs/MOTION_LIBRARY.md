# MOTION_LIBRARY — `video/src/motionlib/`

Biblioteca nova, independente do kit antigo (`video/src/kit/`, usado pela composição `ChannelVideo` do pipeline `tkd`, que não foi alterado).
Importar de `../motionlib`. Frames sempre relativos à `<Sequence>` pai.

## Contrato comum (`core.ts`)
`AnimProps`: `delay`, `durationInFrames`, `x`, `y`, `scale`, `opacity`, `speed`, `easing` (`linear|in|out|inOut|outExpo|inExpo|outBack|[bezier]`), `direction`.
Helpers: `progress(frame, anim)`, `ease(name)`, `smoothNoise(seed, t, hz)`, `glitchAt(seed, frame, p)`.

## Componentes
| Componente | Para quê | Props principais | Origem (ref) |
|---|---|---|---|
| `Shot` | Plano de footage com câmera virtual | `src, startAt, rate, freeze, camera{from,to,easing,delay,speed}, jitter{x,y,freq}, overscan, filter` | ref exp-01: respiração ±10 px; push-in 1→1.043 |
| `CaptionTrack` | Legenda cumulativa palavra/sílaba | `steps[{f,text,stretch}], end, style{fontFamily,weight,italic,sizePx,x,y,color,strokePx,shadow}, twitch{seed,blank,back}, stretch{sx,sy,blurPx}` | ref exp-01: stretch de 1 frame + twitch |
| `TitleStack` | Cartela gigante sílaba a sílaba | `steps, end, fontFamily, capHeight, capRatio, scaleX, scaleY, y, blend, glowPx, glowOpacity, punch` | ref exp-01: difference + glow |
| `levels()` + `Graded` | Grade por plano | `black, white, gamma, saturate, sharpen, tint[r,g,b]` | calibrar com percentis da fonte |
| `FrameFx` | Efeito de frame único | `fx{[frame]: invert|mono|white}` | flash invertido / P&B no drop |
| `LightBloom` | Flare que cresce | `keys[[frame,intensidade]], sources[{x,y,r}], easing` | farol do plano 3 |
| `FilmGrain` | Grão mono | `opacity, size, blend` | |
| `Vignette` | Vinheta | `strength, inner` | |
| `Watermark` | Rodapé fixo | `text, fontFamily, sizePx, y, color, opacity` | |
| `MotionFx` | Eventos de movimento por frame sobre o footage | `events[{f,kind,amount,dur,x?,y?}]`, `glow{amount,threshold,radius}` | ver tabela abaixo e `docs/EFEITOS_AE.md` |
| `Parallax` | Parallax 2.5D / dolly zoom com 2 camadas | `bg, fg, durationInFrames, mode (pan\|dolly), amount, dir` | camadas de `scripts/parallax_layers.py` |

### Eventos do `MotionFx`
| kind | Efeito (equivalente no AE) | `amount` | Curva |
|---|---|---|---|
| `punch` | zoom de impacto no corte | escala inicial (1.12) | decai |
| `pulse` | respiração na batida | escala de pico (1.03) | sino |
| `shake` | tremor | px | decai |
| `rgb` | aberração cromática | px de deslocamento R/B | decai |
| `flash` | clarão branco | opacidade 0..1 | decai |
| `blur` | borrão simples | px | decai |
| `zoomblur` | CC Radial Blur (7 cópias escaladas, média corrente) | zoom extra (0.2) | decai |
| `warp` | Turbulent Displace (feTurbulence + feDisplacementMap) | px | sino |
| `glow` | reforço do Deep Glow (precisa de `look.deepGlow`) | intensidade extra | decai |
| `rays` | CC Light Rays a partir de `x`,`y` (0..1) | intensidade | sino |

## Ainda não existe (próximas fases, só quando uma ref pedir)
Máscaras/recortes em vídeo (o parallax hoje é de um frame parado), partículas, infográficos/números animados, cards, transições não-corte (dissolve/whip), legendas a partir de SRT/ASR (`@remotion/captions` já instalado; `rea-video` exporta `packaging/overlays.ass`).

## Fluxo de validação
`scripts/compare.sh REF REMAKE OUT_DIR [DUR] [tempos...]` → `sbs.mp4`, `pairs.jpg`, contact sheets, faixa da legenda frame a frame, `rea/compare.json`.
Métricas extras usadas no exp-01 (cortes por diff de frame, percentis por plano): o relatório saiu do repositório junto com as análises de referência.

## Composição genérica `recreate` (`video/src/recreate/`)
Props = `RecreatePlan` (`types.ts`), gerado por `tkd recreate`. Tudo dirigido por dados — nenhum canal tem código próprio.

| Campo | Componente | Notas |
|---|---|---|
| `shots[]` | `Shot` dentro de `Graded` | `grade` vem do `scripts/autograde.py`; `transitionIn: {kind:"dissolve", frames}` faz cross-dissolve; `volume` liga o áudio do próprio clipe (padrão mudo); `layers{bg,fg,mode,amount,dir}` troca o vídeo por `Parallax` |
| `motion[]` | `MotionFx` | `scripts/automotion.py` (batidas) + `scripts/fxpass.py` (efeitos AE) |
| `captions[]` + `captionStyle` | `CaptionTrack` | `twitch`/`stretch` null desliga; `maxWidth` quebra linha centralizada; `step.color` sobrescreve a cor (punchline rosa da jane) |
| `title` | `TitleStack` | estilo em `titleStyle` do plano; passos vêm da voz (`kind: "title"`) |
| `frameFx`, `blooms`, `watermark` | `FrameFx`, `LightBloom`, `Watermark` | |
| `look` | `Vignette`, `FilmGrain`, glow | `glow` duplica o plano com blur 8px em screen (levanta as sombras); `deepGlow{amount,threshold,radius}` = Deep Glow só nas partes claras, 3 blurs somados |
| `endCard` | tela sólida com texto | `@handle` no fim (jane) |
| `audio` | `Audio` | `music`+`musicStart`, `voice` (ElevenLabs) |

Fontes por nome: Arimo, ArimoItalic, LeagueGothic, Anton, RobotoCondensed, CormorantItalic, Inter.

Scripts: `measure_ref.py` (cortes/flashes/planos/câmera/música), `aggregate_refs.py` (recorrência entre refs),
`autograde.py` (cor por plano), `voice_lines.py` (ElevenLabs + legendas), `assemble_plan.py` (props), `compare.sh` (validação),
`fxpass.py` / `intro_energy.py` / `parallax_layers.py` / `loudnorm.py` (efeitos AE: ver `docs/EFEITOS_AE.md`).
