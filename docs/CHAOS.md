# TODO — Caos (estilo Chaos Monkey da Netflix)

A Netflix derruba servidores de produção de propósito, ao acaso, para provar que o sistema aguenta falha sem cair
junto. Aqui a ideia é a mesma: matar processos, cortar rede e corromper arquivos **de propósito**, medir o que o
app faz e consertar onde ele quebra feio. O alvo não é "nunca falhar": é **falhar limpo** (sem post duplicado, sem
processo órfão, sem arquivo pela metade usado como se estivesse bom, erro claro na tela, retomada sozinha).

Criado em 18/11/2026.

## Regras de ouro

- **Nunca caos em publicação real.** Experimentos de publicação só com `dryRun` ou conta de teste.
- **Se um teste precisar postar de verdade, o post sai NÃO LISTADO**: no YouTube, `visibility: "UNLISTED"`
  (hoje o `publish.rs` manda `PUBLIC`); no TikTok, que não tem "não listado", "Somente eu". Nunca público.
- **Nada na GPU** (sem `-hwaccel`, sem DirectML): o driver AMD já deu BSOD. Matar o llama-server pode; forçar a GPU não.
- Só na máquina local, com o macaco desligado por padrão. Um experimento por vez, com hipótese escrita antes.
- Todo experimento termina com: Gerenciador de Tarefas sem órfão, `state.db` íntegro, nenhum arquivo `*.tmp`/parcial.

## Fase 0 — o macaco (infra de injeção)

O app já tem um ponto único por onde passam quase todos os filhos (`proc.rs`: `tree_output`/`tree_status`),
então o macaco entra ali, sem espalhar `if` pelo código.

- [x] `TKD_CHAOS` (desligado por padrão), ex.: `TKD_CHAOS="kill:ffmpeg:0.2,delay:claude:30s,fail:yt-dlp:1.0"`
  (`crates/tkd-core/src/chaos.rs`; regra inválida aborta em vez de rodar outro experimento)
  - `kill:<bin>:<prob>[:<max>]` mata a árvore do filho num instante aleatório até `<max>` (padrão 10 s)
  - `crash:<bin>:<prob>[:<max>]` mata **só** o processo direto (testa se os netos morrem junto)
  - `fail:<bin>:<prob>` filho sai com erro sem rodar; `block:<bin>:<prob>` idem com a mensagem 429/"not a bot"
  - `delay:<bin>:<tempo>` atrasa a resposta do filho (simula PC lento / rede lenta)
  - `hang:<bin>` filho nunca termina (testa os timeouts)
  - `<bin>` = executável (`ffmpeg`, `npx`, `node`, `python`, `claude`, `yt-dlp`, `curl`) ou script (`upload`,
    `upload_yt`, `speech`, `eval_video`, `live`); `*` = todos. Entra no `proc.rs` e nos 3 spawns manuais
    (render, agente, navegador).
- [x] Ganchos fora do `proc.rs`: `config::write_atomic` (`fail:write`, `full:write` = disco cheio com temporário
  pela metade, `fail:rename` = OneDrive segurando), `agent.rs` (`empty:claude`, `truncate:claude`,
  `ratelimit:claude` — o 429 responde **antes** de rodar, sem gastar token). Bloqueio do YouTube = `block:yt-dlp`;
  cobalt fora = `fail:curl` (os dois passam pelo `proc.rs`, não precisaram de gancho próprio).
- [x] Semente fixa (`TKD_CHAOS_SEED`) para repetir um experimento que achou bug. Toda injeção vai para o log e para
  `out/chaos/injections.jsonl`.
- [x] `tkd chaos run --scenario <nome> --channel <canal> [--id <video>] [--seed N]`: roda o cenário num tkd filho,
  coleta órfãos/arquivos/estado e grava `out/chaos/<data>-<cenário>.md` (PASSOU / FALHOU / INCONCLUSIVO quando
  o macaco não chegou a atacar). `tkd chaos list` mostra os cenários; `tkd chaos check` mostra órfãos, integridade
  do `state.db`, arquivos parciais e publicações sem confirmação. Render sem `--id` usa uma spec de bancada própria.

## A. Publicação (o mais grave: dano fora do app)

- [x] **App morto entre o upload terminar e o `record_post`** (`publish.rs`): hipótese confirmada pela leitura do
  código (duplicava). Conserto: tabela `publishing` gravada **antes** do upload; `finish_publish` grava o post e
  tira a reserva na mesma transação. Reserva órfã (dono morto ou sem confirmação) → antes de reenviar,
  `browser/check_post.mjs` lê o Studio (só leitura) e decide: achou = grava como postado sem reenviar; lista
  carregada e não achou = libera; dúvida = **não reenvia** e diz como resolver
  (`tkd publish --platform X --mark-posted <url>` ou `--retry-unconfirmed`). O `publish` de qualquer vídeo resolve
  antes as pendências do canal (`reconcile`) e a reserva conta no limite diário. Testado ao vivo
  (`reconcile_ao_vivo`, banco em memória): post existente reconhecido nas duas redes com a URL certa, texto
  inexistente = "absent", uma abertura do navegador por rede.
- [x] Timeout do navegador no meio do upload: mesma checagem. Os scripts mandam `committed: true` a partir do
  clique em publicar; erro antes disso = falha definitiva (libera), depois disso / navegador morto / sem JSON =
  sem confirmação (confere antes de reenviar).
- [x] Sessão do TikTok/YouTube expirada no meio: os scripts devolvem `SESSAO_EXPIRADA` → erro definitivo "conecte o
  TikTok de novo no app", sem retry. A queda fica gravada por rede (tabela `sessions`); com todas as redes do canal
  caídas, o autopilot pula o horário em vez de gastar tokens e render. Volta a valer com login novo ou post ok.
- [x] Dois publish do mesmo vídeo ao mesmo tempo (app + CLI): reserva em transação `IMMEDIATE` no SQLite; o segundo
  recebe "já está sendo publicado (processo N)". Teste `reserva_de_publicacao`.
- [x] Regra de ouro no código: com `TKD_CHAOS` ligado, publicação real no TikTok é recusada e o YouTube sai
  `UNLISTED`. Aberturas seguidas do navegador na mesma rede esperam ≥ 45 s.

## B. Processos filhos

- [x] Matar o Remotion no meio do render: **PASSOU** (`render-kill`, 13 s). Antes o Remotion escrevia direto no
  nome final; agora escreve em `<id>.part.mp4` e só renomeia no fim.
  **Achado no `render-crash`** (mata só o `npx.cmd`): node/Chromium seguiam renderizando e o tkd ficava 46 s
  esperando o pipe fechar. Conserto: quando o filho direto sai, 2 s de folga e a árvore cai
  (`proc::wait_output`, usado no `tree_output`, no agente e no render). Agora 3 s, sem órfão.
- [x] Matar o ffmpeg do loudnorm: já escrevia em temporário e renomeava (conferido); o temporário agora some também
  se a task for cancelada. Recortes do footage mortos no meio não ficam mais largados na pasta de clipes.
- [x] Matar o llama-server no meio da avaliação: a trava é de arquivo do SO (some com o processo) e nota parcial
  não vai para o cache (conferido no código). **Achado:** o cache era gravado com `json.dump` direto; morto no meio,
  ficava truncado e toda avaliação futura daquele vídeo quebrava no `json.load`. Consertado no repo do avaliador
  (branch `fix/atomic-eval-cache`): temporário + `os.replace`, e cache corrompido é descartado e reavaliado
  (testado truncando um cache real).
- [x] Matar o whisper / `speech.py`: **PASSOU** (`speech-kill`): os momentos saem sem fala, nada largado no `%TEMP%`.
- [x] `claude -p` travado (`hang`): **PASSOU** (`agent-hang`, teto encurtado com `TKD_CHAOS_LIMIT`). Resposta
  cortada/vazia (`agent-truncate`, `agent-empty`): tenta 3× e falha claro, sem spec gravada. As respostas falsas
  agora saem antes de abrir o `claude`: nenhum cenário gasta token.
- [x] llama-server morto sob caos de verdade (`eval-llama-kill`, mata pelo nome na árvore do avaliador): falha
  limpa na subida e no meio da geração, e a avaliação seguinte termina. **Achado:** morto no meio da geração saía
  um traceback (`ConnectionResetError`); agora "o modelo caiu no meio da avaliacao; tente de novo".
- [x] `yt-dlp`/`streamlink` travados: **não existiam** no `footage::ytdlp` nem no download do scout. Agora 30 min
  e 15 min. `live_meta` tem 60 s; o streamlink dentro do `live.py` tem watchdog próprio (não conferido sob caos).

## C. Rede

- [x] YouTube 429 / "not a bot": **PASSOU** (`youtube-block`): para na 1ª chamada (sem trocar de client), grava a
  pausa. O estado do YouTube dos experimentos vai para `out/chaos/yt_pace.json` (`TKD_YT_PACE_FILE`): o real fica
  intacto e os canais de verdade não pausam por causa de um teste.
- [x] Internet cai no meio do download: **PASSOU** (`net-drop`, `net-stall`). Servidor local falso
  (`scripts/fake_http.py`) manda 40% do vídeo e sai do ar ou trava a conexão; nada sai na internet. **Achados:**
  (1) o `RemoveOnDrop` só apagava o nome final: os `.part`/`.fNNN.mp4`/`.ytdl` do yt-dlp ficavam no `%TEMP%`,
  e como o nome do temporário se repete (`tkd-src-<canal>-<n>.mp4`), a próxima busca podia retomar o `.part` de
  OUTRO vídeo. Agora `RemoveFamily` limpa o arquivo e os pedaços antes de baixar (sobra de app morto) e ao sair.
  (2) `moments --url` só apagava o download no fim: erro no meio deixava o mp4 no `%TEMP%`. (3) Rede presa dizia
  "instale o yt-dlp"; agora "yt-dlp parado sem receber nada: rede caiu". (4) `moments --url` com link direto
  (não YouTube) falhava em "Requested format is not available"; o formato tem `/b` de último recurso.
- [x] Todas as instâncias do cobalt fora: **PASSOU** (teste `todas_fora_falha_rapido_e_limpo`: porta fechada,
  erro, captcha, HTML e túnel vazio): erro em 0,1 s listando cada uma, sem arquivo largado, e na próxima nem pede
  de novo (instância fica 1 h de lado). O footage cai para o yt-dlp com esse erro no log.
- [ ] ElevenLabs fora (TTS): edit sai com fala real ou falha claro; nunca "vídeo sem voz e sem aviso". ( ter cuidado com isso nao é todo video que isso é usado, é so em casos extremos de canais no futuro nao agora )

## D. Disco e estado

- [x] Disco cheio de verdade: **disco virtual de 12 MB** (`scripts/chaos_disk.ps1`, pede admin; arquivo em
  `%LOCALAPPDATA%\tkd-chaos`, fora do OneDrive) com o `TEMP` do filho nele. A bancada recusa disco com mais de
  100 MB livres (nunca enche o disco do sistema).
  - `disk-full-download` **PASSOU**: `[Errno 28] No space left on device` no meio do download, nada largado.
  - `disk-full-render` **2 achados**: (1) o tkd entrava em **pânico** (`async fn resumed after completion`) quando o
    stdout do Remotion fechava antes do processo sair (erro rápido): agora sai "remotion render falhou: ENOSPC...".
    (2) render morto/falho largava perfil do Chromium, bundle e assets soltos no `%TEMP%` (a bancada não via: só
    olhava `tkd-*`). Agora cada render tem `TEMP` próprio (`tkd-render-<id>-<pid>`), apagado na saída (tenta por
    até 3 s enquanto o Windows solta os arquivos) e varrido no próximo render se o processo morreu. Com 12 MB o
    disco enche ainda no bundle; o teste "no meio dos quadros" pede um disco de 64 MB
    (`chaos_disk.ps1 -SizeMB 64`), não rodado (UAC cancelado).
- [x] OneDrive segurando `clips.json` / `state.db` (rename falha): o `write_atomic` não tentava de novo e largava
  o `.tmp`. Agora tenta 5× com espera crescente, apaga o temporário em qualquer falha e o original fica intacto.
  `clips.json`, `tracks.json`, `edits.json`, `names.json` e o histórico de cortes passaram a usar a mesma escrita.
  O `state.db` segue no OneDrive (decisão pendente de tirar os dados de lá); travamento dele não foi testado.
- [x] `clips.json` corrompido (JSON truncado): **bug real** — `load_bank` devolvia lista vazia calado e o próximo
  `save_bank` gravava o banco vazio por cima de tudo. Agora `config::load_json` guarda o ruim em
  `<arquivo>.corrupt-<data>`, volta a última versão boa (`.bak`, gravado a cada `save_json`) e avisa no log.
- [x] `state.db` travado por outro processo: **2 achados**. (1) A espera era 10 s: OneDrive segura mais que isso;
  agora 30 s (`db-locked` 6 s e `db-locked-long` 20 s seguem sem erro) e, passando disso, "state.db ocupado por
  outro programa (OneDrive?): tente de novo" (`db-locked-forever`) em vez de "database is locked". (2) O gravador
  de eventos desistia para sempre se o banco estivesse travado ao abrir e jogava fora lote que falhava: agora
  guarda em memória (até 20 mil) e grava quando o banco volta; na saída insiste até 30 s.

## E. App e PC

- [x] Fechar o app no meio de um edit: **PASSOU** (`app-killed-mid-run`: mata o próprio tkd aos 25 s com tudo
  caro travado): filhos morrem junto (Job Object), o run fica aberto e `close_open_runs` marca interrompido.
- [x] PC hiberna durante uma live: **PASSOU** (`live-sleep`: a árvore inteira do tkd congelada 3 min com
  `NtSuspendProcess`, como na hibernação). Ao acordar o `live.py` vê "captura parada há 208 s", sai, e o vigia
  reabre em 26 s; voltou a gravar e a cortar, nada órfão, nenhum clipe sobrescrito.
- [x] Reboot no meio do autopilot: **2 achados** (teste `piloto_depois_do_reboot_nao_dobra_nem_pula`, que falha
  com o código antigo). (1) **Pulava post**: `posts_since` contava linhas de `posts`, que tem uma linha por rede;
  um vídeo no TikTok + YouTube contava 2 e o 2º horário do dia sumia. (2) **Dobrava post**: a reserva de upload
  (`publishing`/`unconfirmed`) não entrava na conta; depois de um reboot no meio do upload o mesmo horário fazia
  e postava outro vídeo. Agora conta vídeos distintos de posts + reservas; reserva liberada (conferência disse
  "não foi") devolve o horário. O reboot físico não foi feito: quem decide na volta é só o banco (as tentativas
  em memória zeram, o que só dá mais tentativas, nunca mais posts).
- [x] App e CLI ao mesmo tempo: **achado** — as travas de navegador e "canal ocupado" eram só dentro do processo.
  Agora também entre processos (`.profiles/.browser.lock`, `out/.busy/<canal>.lock`): `browser-lock` (a CLI espera
  o app soltar o navegador) e `channel-busy` (o segundo recusa na hora) PASSARAM. A da GPU já era de arquivo.

## F. Live

- [x] Live cai e volta: **achado** (`live-reopen`, com o vigia do piloto de verdade via `tkd chaos watch-live`;
  o streamlink falso fica fora do ar por 60 s por um arquivo de estado). A captura fecha limpa, mas o vigia via
  "offline" durante a queda e só conferia de novo 3 min depois: reabriu **210 s** depois da volta (+90 s de
  aquecimento = ~5 min sem corte a cada queda). Agora, nos 10 min depois de uma captura acabar, confere a cada
  1 min (2 no YouTube): reabriu em 90 s. Também: pasta de live reaberta no mesmo minuto sobrescrevia `clip_01`;
  agora a numeração continua. Nenhum clipe duplicado ou sobrescrito nos testes.
- [x] Segmento corrompido no buffer: **PASSOU** (`live-garbage`, 256 KB de lixo no meio do TS): o ffmpeg reclama
  e segue, 4 clipes, termina no tempo pedido.
- [x] Chat cai (Twitch e YouTube): **achado**. Chat falso local (`scripts/fake_chat.py`: IRC da Twitch e
  `live_chat` do YouTube, via `TKD_TWITCH_IRC`/`TKD_YOUTUBE_BASE`) cai aos 75 s. A live seguia, mas o chat morto
  entrava na nota com valor ~0 e peso 0,40: depois da queda nenhum pico passava do limite (0 cortes; o mesmo
  vídeo com `--no-chat` dava 4). Agora chat com erro e mudo há 60 s sai da nota ("chat fora do ar: notas só com
  áudio/movimento") e volta quando reconecta (o erro antigo nunca era limpo). `live-chat-drop-twitch` e
  `live-chat-drop-youtube` PASSARAM: 2 cortes depois da queda em cada.
- [x] Chat da Kick: **2 achados** (API falsa local, `TKD_KICK_BASE`). (1) 429 não marcava erro: o chat ficava
  mudo até 5 min e seguia contando na nota (achado lendo o código; o teste não foi rodado sem o conserto).
  (2) Sucesso não limpava erro antigo: uma falha passageira deixava o chat "caído" para sempre. `live-chat-drop-kick`
  (API sai do ar) e `live-chat-429-kick` (429 para sempre) PASSARAM: cortes seguem sem o chat. Os cenários de live
  sem chat falso apontam os três chats para uma porta fechada (antes os de reabrir/dormir conectavam no IRC real
  da Twitch, anônimo e só leitura).

## G. Dados ruins (caos na entrada)

- [x] Sem áudio, 0 bytes, 1 frame: **achados**. `tkd moments` aceitava arquivo vazio/1 frame e gravava "0 s de
  vídeo, 0 picos" como se fosse bom; agora recusa (< 1 s) logo no começo. O avaliador quebrava num traceback do
  soundfile com 1 frame; agora recusa vazio, ilegível e curto antes de subir o modelo. Sem áudio já era recusado.
- [x] Agente devolve clipe inexistente / fala inexistente: **achado**. Fala inexistente e voiceClip inexistente
  já voltavam para o agente, mas shot com clipe inexistente passava calado no `check_plan` e o recreate inteiro
  morria depois no `validate`, sem o agente poder corrigir. Agora é erro de correção (teste em
  `plano_conferido_no_catalogo`).
- [x] Banco sem nenhum clipe do personagem: **achado**. Só avisava, analisava a ref e mandava "(banco vazio)" para
  o agente (tokens num plano impossível). Agora recusa na hora mandando rodar `footage` (`recreate-no-character`,
  0 s). Fraqueza anotada: `is_character` casa qualquer palavra de 3+ letras do nome, então um nome com
  "Que"/"Nao" casa com descrições em português.

## Ordem sugerida

1. **A (publicação):** é o único dano que sai do app e não dá para desfazer.
2. Fase 0 mínima (`kill` e `hang` no `proc.rs`) + B.
3. E e D (fechamento do app, disco, OneDrive).
4. C, F e G.

## Resultados — 18/11/2026

Relatórios em `out/chaos/`. Todos terminaram sem processo órfão, `state.db` íntegro e sem arquivo pela metade.

| Cenário | Resultado | O que mostrou |
|---|---|---|
| `render-kill` | PASSOU (13 s) | mp4 final intacto; Remotion agora renderiza em `.part.mp4` |
| `render-crash` | FALHOU → consertado → PASSOU (3 s) | netos seguiam vivos 46 s depois do `npx.cmd` morrer |
| `publish-dry-kill` | PASSOU (42 s) | Edge do perfil fechado, nenhuma reserva pendurada. O YouTube deixa um rascunho privado quando morre depois de escolher o arquivo (já era assim no dry-run) |
| `agent-ratelimit` | FALHOU → consertado → PASSOU (0 s) | o próprio macaco injetava o 429 depois de o `claude` rodar (gastava uma chamada) |
| `reconcile_ao_vivo` (teste ignorado) | PASSOU | conferência do Studio acha o post certo nas duas redes e diz "absent" para texto inexistente |

| cache do avaliador truncado | FALHOU → consertado → PASSOU | reavalia em vez de quebrar para sempre |

Ainda não rodados: `publish-dry-hang`, `agent-truncate` (gasta uma chamada), B4/B5 sob caos,
C, E, F e G. Próximo: E (fechar o app no meio de um edit) e D1/D4.

## Resultados — 18/11/2026 (2ª rodada)

`tkd chaos list` mostra os 23 cenários. Todos os abaixo terminaram sem processo órfão, `state.db` íntegro, nada pela
metade e nada novo no `%TEMP%`. Nenhum publicou, gastou token, usou a GPU forçada ou saiu na rede (o único que abre
navegador, `publish-dry-kill`, ficou da 1ª rodada).

| Cenário | Resultado |
|---|---|
| `agent-hang` / `agent-truncate` / `agent-empty` / `agent-ratelimit` | PASSOU (15 s / 0 s / 0 s / 0 s) |
| `db-locked` / `db-locked-long` | FALHOU (10 s) → espera 30 s → PASSOU |
| `db-locked-forever` | PASSOU (mensagem clara aos 32 s) |
| `channel-busy` / `browser-lock` | PASSOU (trava entre processos nova) |
| `publish-dry-hang` | PASSOU (teto encurtado; mensagem dizia "0 min", agora em segundos) |
| `app-killed-mid-run` | PASSOU |
| `speech-kill` | PASSOU |
| `youtube-block` | PASSOU |
| `eval-llama-kill` (subida e geração) | PASSOU (traceback virou mensagem) |
| `live-drop` / `live-garbage` | PASSOU (streamlink falso) |
| `bad-*-moments` / `bad-*-eval` | FALHOU → validação na entrada → PASSOU |
| `render-kill` / `render-crash` (de novo) | PASSOU |

Rodada 3 (mesmo dia): `net-drop`, `net-stall`, `recreate-no-character` PASSOU; cobalt fora PASSOU no teste.
D1 pronto, esperando o disco de 12 MB (admin). Fora da bancada: ElevenLabs (C4, adiado), hibernar/reboot (E2/E3),
chat caindo na live (F3).

Rodada 4: `live-chat-drop-twitch`/`-youtube`, `disk-full-download`, `disk-full-render`, `render-kill`/`-crash`
(de novo, com o `TEMP` próprio do render) PASSARAM; reboot do autopilot coberto por teste. Fica de fora só o
ElevenLabs (C4, adiado), hibernar (E2) e o disco de 64 MB para encher no meio dos quadros.

Rodada 5: `live-reopen` (FALHOU 210 s → consertado → PASSOU 90 s), `live-sleep`, `live-chat-drop-kick`,
`live-chat-429-kick` PASSARAM. Live completa nas três plataformas. Fica de fora só o ElevenLabs (C4, adiado) e o
disco de 64 MB para encher no meio dos quadros do render.
