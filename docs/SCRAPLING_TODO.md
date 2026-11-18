# TODO: Scrapling no tkd

Plano para usar o [Scrapling](https://github.com/D4Vinci/Scrapling) (`D4Vinci/Scrapling`, PyPI `scrapling`)
em tudo que hoje acessa a web. Auditado em 17/11/2026 (commit `2998c9e`): sem sinal de malware, núcleo com poucas
dependências; o extra `fetchers` traz `curl_cffi`, `playwright`, `patchright` e `browserforge`.

Regra geral: o Scrapling entra onde a gente **lê** página ou API pública sem conta. Onde precisa da **sessão logada**
de um canal (login, upload, studio) ou onde já existe ferramenta especializada (yt-dlp, streamlink, IRC), fica como está.
Ele não fura 429 por IP: o `yt_pace` e o proxy continuam valendo.

## 0. Instalação

- [x] Instalar no venv que o Rust já usa (`tools/rea-video/.venv`, via `recreate::venv_bin`):
      `pip install "scrapling[fetchers]==<versão fixa>"` + `scrapling install` (baixa os navegadores do patchright).
- [x] Fixar a versão no requirements do venv; não usar `latest`. → `scripts/requirements-web.txt` (0.5.1).
- [x] Usar o Edge instalado em vez do Chromium baixado. `real_chrome=True` só acha o Chrome (não instalado aqui), então
      `scripts/web.py` passa `executable_path` do `msedge.exe` (`TKD_EDGE` sobrescreve).
- [x] Teste de fumaça (17/11): `Fetcher.get` no perfil do TikTok **cai no WAF** (página SlardarWAF de 1.4 KB), então
      HTTP puro não serve para TikTok. `StealthySession` headless no Edge passou (31 vídeos + `/api/post/item_list/`).
      Depois de ~8 aberturas em 5 min o IP tomou muro suave: perfil abre, grade vazia, `item_list` 200 com corpo vazio
      (headless, janela e perfil novo, todos iguais). Não martelar.
- [ ] Não usar o MCP do Scrapling nem a `agent-skill/` sem ler o `SKILL.md` antes (texto feito para agente, risco de prompt injection).

## 1. Migrar (ganho claro)

### 1.1 `scripts/live.py`: Kick (`kick_info`, `chat_kick`)
Hoje usa `requests` puro em `kick.com/api/v2/channels/...`. O Kick fica atrás do Cloudflare e `requests` tem
fingerprint TLS de Python, o que causa 403/429 antes da hora.
- [x] Trocado por `web.http_session()`: `curl_cffi.Session(impersonate="chrome")` (o motor do `FetcherSession`), com a
      API do `requests`, então o loop não mudou. Sem curl_cffi cai no `requests`.
- [x] Manter o poll adaptativo 0.7–5 s e o contador `n429`.
- [ ] Se aparecer desafio do Cloudflare: pegar os cookies uma vez com `StealthyFetcher(solve_cloudflare=True)` e
      reaproveitar na sessão HTTP.

### 1.2 `scripts/live.py`: chat do YouTube (`youtube_id`, `chat_youtube`)
Hoje usa `requests` em `live_chat` e `youtubei/v1/live_chat/get_live_chat`.
- [x] Mesma troca (`web.http_session`).
- [x] Continua dividindo o orçamento por IP com o yt-dlp (`yt_pace`); o chat cede quando apertar.

### 1.3 `browser/scrape_channel.mjs`: lista de vídeos do perfil de referência (`scout`)
Hoje abre o Edge com Playwright no perfil logado só para ler a grade pública.
- [x] Novo `scripts/tiktok_web.py channel` (um arquivo só para channel/download/comments) com `StealthySession` (`real_chrome=True`, `hide_canvas=True`,
      scroll via `page_action`) lendo a grade de `tiktok.com/@<handle>`, com a mesma saída JSON do `.mjs`.
- [x] ~~Alternativa mais leve: `Fetcher` + parse do JSON embutido~~ (WAF barra o HTTP puro; o JSON embutido é lido dentro do navegador) (`__UNIVERSAL_DATA_FOR_REHYDRATION__`); testar se
      basta antes de subir navegador.
- [x] `scout.rs` (`listar`): `browser::run_web("channel")` pelo venv; erro/muro cai no `scrape_channel.mjs`.
- [x] Vantagem: o scout para de disputar o semáforo único do navegador (`browser.rs`) com login e upload.

### 1.4 `browser/download.mjs`: download dos vídeos de referência
Hoje clica no thumbnail e captura a URL do CDN na resposta de rede (`waitForResponse`).
- [x] Primeiro tentar `yt-dlp` direto no link do TikTok (já é a ferramenta de download do projeto).
- [x] Se o yt-dlp falhar: `tiktok_web.py download` (playAddr do `item_list`; clique no thumbnail se o item não veio na API).
- [x] `scout.rs` (`baixar`): ordem yt-dlp → Scrapling → `.mjs` antigo; cada etapa só recebe o que falhou.

### 1.5 `browser/comments.mjs`: comentários públicos (dataset do avaliador)
Hoje abre cada vídeo e captura `/api/comment/list/`.
- [x] Reescrever com `StealthySession(record_requests=True)` (uma sessão, uma aba por vídeo), lendo
      `/api/comment/list/` em `session.network`.
- [x] Manter as regras atuais: parar em `has_more=0`, cursor repetido ou 2 rolagens sem novidade; prazo de 45 s;
      `collapse()` por texto com `dupes`.
- [x] Mesmo formato de saída `<outDir>/<id>.comments.json`. `dataset_*.sh` chamam o `.py` e caem no `.mjs` se falhar.

### 1.6 `browser/lib.mjs`: `avatarTikTok` e o fallback `fetch()` do `whoamiYT`
- [x] ~~Avatar público via `Fetcher`~~: não migrado. `avatarTikTok` lê a página já aberta no Edge logado (login.mjs), e
      HTTP puro no TikTok cai no WAF.
- [x] O `fetch()` do canal do YouTube virou `getPublic()` → `scripts/web.py get` (curl_cffi); sem venv cai no `fetch()`.

## 2. Avaliar depois (talvez)

- [ ] **Busca de música e trends** (fase futura do avaliador): nasce já em Scrapling (`Fetcher` primeiro,
      `StealthyFetcher` se tiver muro).
- [ ] **Metadados de canais e VODs** da Twitch/Kick no `cortes.rs` e `live.rs`, se algum dia sair do yt-dlp/streamlink.
- [ ] **Spiders do Scrapling** (`scrapling.spiders`, com cache) para o `scout` de vários perfis em lote, se o número de
      canais passar de ~10.

## 3. Não migrar (fica como está)

| Onde | Por quê |
|---|---|
| `browser/login.mjs`, `whoami.mjs`, `isLoggedIn*` | Precisa do perfil persistente `.profiles/<canal>/` com a conta logada |
| `browser/upload.mjs`, `upload_yt.mjs` | Upload pelo TikTok Studio e YouTube Studio logados; trocar o motor aumenta o risco de ban da conta |
| `footage.rs` (`ytdlp`, `ytsearch3`), `music.rs`, `clip_quality.py`, `requalify_clips.py` | O yt-dlp com PO token (bgutil) é o certo para o YouTube; o Scrapling não baixa vídeo nem gera PO token |
| `cobalt.rs` | Baixa pelo IP da instância, que é justamente o que ajuda contra 429 |
| `live.py`: captura (`streamlink`) e chat da Twitch (IRC anônimo) | Protocolo próprio, sem anti-bot a contornar |
| `voice_lines.py` (ElevenLabs) | API oficial com chave |

## Armadilhas achadas

- `curl_cffi` não abre o `cacert.pem` num caminho com acento ("Área de Trabalho") quando o Python roda em modo UTF-8
  (erro 77). `web.py` passa o nome curto 8.3 (ou cópia no temp).
- `python -I` ignora `PYTHONUTF8`; os chamadores usam `-X utf8`.
- `page_action` do Scrapling roda **antes** do `wait_selector`, e os erros dentro dele só viram log. Por isso a espera e
  os erros ficam dentro da própria ação.
- O `NetworkRecorder` guarda no máximo 1 MB por resposta: serve para JSON, não para mp4.

## 4. Critério de pronto

- [ ] Scout de 1 canal completo (lista + download + comentários) sem abrir o Edge logado.
- [ ] Taxa de muro anti-bot igual ou menor que a do Playwright em 3 dias seguidos (logar no `TKD_LOG=debug`).
- [ ] Kick: 1 live inteira sem 403 no chat.
- [ ] Remover os `.mjs` substituídos e a dependência deles no `browser.rs` só depois disso.
