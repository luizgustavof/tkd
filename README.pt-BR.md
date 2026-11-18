# tkd — fábrica de canais TikTok / Shorts

> [English](README.md) · **Português**

App em Rust (CLI `tkd` + app desktop Tauri) que roda canais de vídeo curto sozinho. Cada canal é um agente com
memória própria, conta própria (um perfil do Edge por canal) e horários de post próprios.

Dois formatos de canal (`format` no `channel.toml`):

- **edit**: estuda perfis de referência, recria a estrutura de um edit com footage do banco do canal e a fala real
  do personagem, renderiza no Remotion, avalia (QA medido + Claude) e publica.
- **clips** (cortes de live; `"cortes"`, o nome antigo, ainda vale): vigia lives de streamers (Twitch, Kick, YouTube), corta os momentos fortes pelo chat/áudio/movimento,
  confere a categoria da live (jogo do canal), escolhe o melhor corte com o Claude, faz o 9:16 com legenda e publica
  com crédito ao streamer.

## Instalar

**Jeito fácil (Windows):** baixe o projeto e dê duplo clique em `instalar.bat`. Passo a passo em
[`INSTALAR.md`](INSTALAR.md). Conta, modelo e gasto do Claude: [`CLAUDE_CODE.md`](CLAUDE_CODE.md).

## Pré-requisitos (instalação manual)

Node 20+, Rust, pnpm, ffmpeg no PATH, Microsoft Edge, CLI `claude` logada.

- **yt-dlp** no Python global com o solucionador JS do YouTube: `pip install -U "yt-dlp[default]"` (traz o
  `yt-dlp-ejs` na versão certa; sem ele os formatos HD somem). Atualize sempre com esse comando.
- **venv `tools/rea-video/.venv`** (whisper, streamlink, Scrapling): `pip install -r scripts/requirements.txt` e
  depois `scrapling install`. Ver `docs/SCRAPLING_TODO.md`.
- **rea-video** (cortes, falas e stems do formato edit): clone o
  [JuneYaooo/rea-video](https://github.com/JuneYaooo/rea-video) em `tools/rea-video` e rode
  `pip install -e "tools/rea-video[asr]"` no venv. Os cortes de live funcionam sem ele.

Tudo roda na CPU: o ffmpeg não usa `-hwaccel` nem DirectML.

## Primeiro canal

```bash
cp tkd.example.toml tkd.toml                          # modelos do Claude por tarefa, piloto automático
cp -r channels/_example channels/meu-canal            # edite slug, nicho, persona, horários
```

Ou crie pelo app (tela **Canais** → novo canal) e conecte o TikTok/YouTube por lá. A tela **Primeiros passos**
(ou `tkd doctor`) mostra o que falta instalar e o comando de cada conserto.

## App desktop

```bash
cd app
pnpm install
pnpm tauri build --debug --no-bundle   # gera target/debug/tkd-app.exe
```

Idioma da interface: português, inglês ou árabe (da direita pra esquerda), no seletor no rodapé do menu. Traduções
em [`app/src/lib/messages.ts`](app/src/lib/messages.ts).

Telas: **Canais** (criar, configurar, conectar TikTok/YouTube, horários, observações), **Ao vivo** (o que cada
agente faz agora, uma linha por live vigiada), **Agentes** (modelo e esforço por tarefa), **Cortes** (tudo que o
clipper cortou, com prévia, player, filtro e apagar) e **Consumo** (tokens por canal/tarefa).

O piloto automático (`[app] autopilot` no `tkd.toml`) liga os vigias de live e o ciclo de post. Fechar o app no
meio é seguro: os filhos morrem junto (Job Object) e a próxima abertura marca o que ficou aberto como interrompido.

## CLI

| Comando | O que faz |
|---|---|
| `status` | Estado de cada canal |
| `doctor` | Confere ffmpeg, node, `claude`, yt-dlp, venv, Edge e config (o mesmo da tela **Primeiros passos**) |
| `login` / `logout` / `accounts [--check]` | Sessão do TikTok/YouTube no perfil do canal (`.profiles/<canal>/`) |
| `yt-login` | Conta Google **descartável** para os cookies do yt-dlp (nunca a conta de um canal) |
| `scout` / `analyze` / `study` | Baixa e mede as referências; destila `style-card.md` + `preset.json` |
| `footage` / `catalog` / `moments` / `music` | Banco de clipes do canal (busca, recorte, catálogo por cena, trilha) |
| `recreate` | Recria um edit a partir de uma referência |
| `ideate` / `render` / `review` / `publish` | Etapas soltas do pipeline |
| `run --channel X [--dry-run] [--test-post]` | Pipeline completo de um vídeo |
| `live --url URL` | Assiste uma live e corta os momentos fortes (sem o piloto) |
| `watch` | Ciclo autônomo pela CLI (o mesmo do piloto do app) |
| `usage` | Consumo de tokens |
| `chaos check / list / run / watch-live` | Testes de caos (`docs/CHAOS.md`) |

- `--dry-run`: vai até a tela de publicar e não clica.
- `--test-post`: publica de verdade, mas **YouTube não listado** e **TikTok "Somente você"**. O script confere o
  seletor de privacidade antes de clicar e aborta se não conseguiu marcar.
- `TKD_LOG=debug` liga o log detalhado. `TKD_MAX_WATCH` (padrão 6) muda quantas lives são vigiadas juntas.

## Estrutura

```
src/              CLI (clap)
crates/tkd-core/  todo o motor: canais, agentes, pipeline, cortes, live, publish, eventos, state.db
app/              app desktop (Tauri 2 + React; bindings gerados com `pnpm bindings`)
browser/          Playwright no perfil logado: login, upload TikTok/YouTube
scripts/          Python: live.py (clipper), clip_finish.py, qa.py, speech.py, moments.py...
agents/           templates de prompt do Claude (ideate, recreate, review_edit, review_clips...)
video/            projeto Remotion (composições e `motionlib`)
channels/<slug>/  channel.toml, knowledge/ (style-card, preset, banco de clipes), library/
out/              specs, renders, recreate, live/<sessão>/clips (gerado)
docs/             testes de caos, regras de qualidade, efeitos do Remotion
```

Próximos passos: [`ROADMAP.md`](ROADMAP.md).

Canais com prefixo `_` (ex.: `_example`) ficam fora do piloto e só rodam chamados pelo slug.

## Avaliação dos edits

Antes de postar, cada edit passa pelo QA medido (`scripts/qa.py`: loudness, pico, tela parada, legenda) e pela
review do Claude. Dá para ligar **o seu próprio modelo local**: qualquer comando que recebe o mp4 e imprime uma
nota em JSON (`[evaluator]` no `tkd.toml`, protocolo em [`docs/EVALUATOR.md`](docs/EVALUATOR.md)). Nota clara
decide sozinha; nota duvidosa vai para o Claude junto com a avaliação do modelo. O tkd não traz modelo nenhum; sem
ele, a review segue só com o QA e o Claude.

## Avisos

- A automação usa a sessão real do navegador e contraria os termos do TikTok: existe risco de bloqueio. Os limites
  (posts por dia, pausas, um navegador por vez, ≥ 45 s entre aberturas do TikTok, orçamento de 200 req/h no YouTube)
  reduzem, não eliminam.
- O upload confere o `@handle` logado contra o `post_handle` do canal e aborta se forem diferentes.
- `.profiles/` tem os cookies de cada conta: não versionar nem compartilhar (está no `.gitignore`).
- Vídeos baixados (refs, footage, lives) são material de trabalho local. Corte de live sai com crédito ao streamer.

## Licença

Código sob [MIT](LICENSE): pode usar, modificar e vender, inclusive em produto comercial, mantendo o aviso de
copyright.

Dependências com licença própria, que valem à parte:

- **Remotion** (render dos edits): grátis para pessoa física e empresa pequena; empresa maior precisa da
  [licença da Remotion](https://www.remotion.dev/license). Quem só usa o formato cortes (ffmpeg) não depende dele.
- **Modelos e serviços** (Claude, whisper, ElevenLabs): cada um com os próprios termos.
- **Plataformas**: a automação de post contraria os termos do TikTok (ver Avisos). Vídeo de terceiro (footage,
  live) segue os direitos de quem fez.
