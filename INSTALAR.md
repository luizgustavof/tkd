# Instalar o tkd

Para Windows 10 ou 11. Não precisa saber programar.

**Precisa ter:** uns 20 GB livres no disco, internet e uma conta do Claude (plano Pro ou Max, ou uma chave de API).
**Tempo:** 30 a 60 minutos na primeira vez, quase tudo esperando o download e a compilação.

## 1. Baixar

No GitHub, clique no botão verde **Code** → **Download ZIP**. Extraia o ZIP numa pasta fácil, por exemplo
`C:\tkd`.

> Evite pastas sincronizadas (OneDrive, Google Drive): o banco de dados e os vídeos mudam o tempo todo e a
> sincronização atrapalha.

Quem já usa git pode clonar: `git clone <endereço do repositório> C:\tkd`.

## 2. Instalar

Abra a pasta e dê **duplo clique em `instalar.bat`**.

- Se o Windows mostrar "O Windows protegeu o computador", clique em **Mais informações** → **Executar assim mesmo**.
- O instalador mostra cada passo. Quando pedir permissão de administrador (para o compilador C++), aceite.
- Ele só instala o que falta: Git, Node.js, FFmpeg, Python 3.12, Rust, o compilador C++ da Microsoft, pnpm, Claude
  Code, yt-dlp, os pacotes do projeto. Depois compila o app e cria o atalho **tkd** na área de trabalho.

Se terminar com erro, **feche a janela e dê duplo clique de novo**. Na maioria das vezes é só o Windows que ainda
não enxerga um programa recém-instalado; na segunda rodada ele pula o que já está pronto.

## 3. Entrar no Claude

Abra o **Prompt de Comando** (tecla Windows, digite `cmd`, Enter) e digite:

```
claude
```

Siga o login no navegador e feche a janela quando terminar. Detalhes (plano Pro, Max ou chave de API, qual modelo usar, gasto): [`CLAUDE_CODE.md`](CLAUDE_CODE.md).

## 4. Abrir o app

Dê duplo clique no atalho **tkd** da área de trabalho e vá em **Primeiros passos** no menu. Tudo deve estar verde;
o que não estiver tem o comando para copiar ou um botão **Consertar**.

## 5. Primeiro canal

1. **Novo canal** no menu: nome, nicho e formato (**Cortes de live** ou **Edit**).
2. Abra o canal e conecte a conta do TikTok e/ou do YouTube. Abre um Edge só daquele canal; faça o login nele.
3. Ajuste os horários de post.
4. Antes de deixar sozinho, clique em **Post de teste** na tela do canal (e de novo em **Confirmar**). Ele faz um
   vídeo e publica de verdade, mas só você vê: **"Somente você"** no TikTok e **não listado** no YouTube. Não gasta
   os posts do dia. Pela CLI: `target\release\tkd.exe run --channel meu-canal --test-post`.
5. Ligue o **piloto automático** no canto de baixo do menu.

## Atualizar

Baixe a versão nova por cima (ou `git pull`) e dê duplo clique em `instalar.bat` de novo. Seus canais, contas e
configurações ficam: estão em `channels/`, `.profiles/`, `tkd.toml` e `state.db`, que o instalador nunca apaga.

## Problemas comuns

| O que aparece | O que fazer |
|---|---|
| `winget não encontrado` | Instale o **Instalador de Aplicativo** pela Microsoft Store e rode de novo |
| `link.exe not found` ou erro de compilação | O compilador C++ não terminou: rode o `instalar.bat` de novo |
| yt-dlp sem o solucionador / vídeos só em baixa qualidade | Em **Primeiros passos**, botão **Consertar** no yt-dlp |
| O app abre e fecha | Rode `instalar.bat` de novo; se continuar, abra um terminal na pasta e rode `target\release\tkd.exe doctor` |
| Login do TikTok pede captcha toda hora | Normal no começo; resolva no Edge do canal. Não abra o TikTok várias vezes seguidas |

Opções do instalador pelo terminal: `powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -DryRun` (só
mostra o que faria), `-SkipBuild` (não compila), `-NoShortcut` (sem atalho).
