# Configurar o Claude Code para o tkd

O tkd usa o **Claude Code** (o programa `claude`) para todas as decisões criativas: a ideia do vídeo, a montagem do
edit, a legenda e a escolha do melhor corte de live. O `instalar.bat` já instala o programa. Falta entrar na conta e,
se quiser, ajustar modelo e gasto.

## 1. Escolher como pagar

| Opção | Como cobra | Para quem |
|---|---|---|
| **Plano Pro** | mensalidade, com limite de uso a cada poucas horas | 1 ou 2 canais, testando |
| **Plano Max** | mensalidade maior, limite bem mais alto | vários canais no piloto automático |
| **Chave de API** (console da Anthropic) | por uso (tokens), sem limite de horário | quem quer pagar só o que usar |

Com Pro ou Max, quando o limite estoura a tarefa falha e o piloto tenta de novo no próximo ciclo (até 2 tentativas
por horário de post, por até 3 horas). Se o limite durar mais que isso, aquele horário fica sem post; o próximo sai
normal.

## 2. Entrar na conta

Abra o **Prompt de Comando** (tecla Windows, `cmd`, Enter) e digite:

```
claude
```

Na primeira vez ele pergunta como entrar:

- **Plano Pro/Max:** escolha a conta do Claude e faça o login no navegador que abrir.
- **Chave de API:** escolha a opção do console e faça o login lá (ou defina a variável de ambiente
  `ANTHROPIC_API_KEY` com a sua chave).

Quando aparecer a tela de conversa, está pronto. Digite `/exit` para sair.

> A chave de API é uma senha: não cole em arquivo do projeto, print nem issue do GitHub.

## 3. Conferir

No mesmo terminal:

```
claude -p "responda só ok"
```

Tem que aparecer `ok` (gasta uma fração de centavo). No app, a tela **Primeiros passos** mostra o Claude Code em verde
quando acha o login.

Se ele disser que o login não foi encontrado mas o comando acima funcionou, está tudo certo: alguns logins ficam no
cofre de senhas do Windows, que a conferência não lê.

## 4. Modelo e esforço

Na tela **Agentes** do app, ou no `tkd.toml` na pasta do projeto:

```toml
[agents]
model = "claude-opus-5-5"    # padrão para todas as tarefas
effort = "medium"            # low, medium, high: mais esforço = melhor e mais caro
parallel = 2                 # quantas chamadas ao Claude ao mesmo tempo (1 a 8)

[tasks.caption]              # dá para trocar por tarefa
model = "claude-sonnet-5-5"
effort = "low"
```

Tarefas: `ideate` (ideia), `recreate` (montar o edit), `review` (avaliar o edit), `study` (estudar referências),
`footage` (buscar imagens), `tag_clips` (catalogar clipes), `caption` (legenda), `review_clips` (escolher o corte).

| Modelo | Quando usar |
|---|---|
| **Opus** | o mais forte; bom para `recreate`, `study`, `ideate` |
| **Sonnet** | rápido e bem mais barato; bom para `review_clips`, `caption`, `review` |
| **Haiku** | o mais leve; bom para `tag_clips` |

**No plano Pro**, use Sonnet como padrão e Opus só em `recreate`, senão o limite acaba rápido. Com vários canais,
deixe `parallel = 1` ou `2`.

Cada canal também pode ter o próprio modelo (tela do canal → agente). A ordem é: tarefa do canal > canal > tarefa
global > padrão global.

## 5. Acompanhar o gasto

A tela **Consumo** mostra os tokens por canal e por tarefa, e quanto veio do cache (bem mais barato). Pela CLI:
`target\release\tkd.exe usage --days 7`.

## Suas configurações pessoais do Claude Code não atrapalham

O tkd chama o `claude` isolado: sem as suas configurações (`settings.json`, hooks, plugins), sem servidores MCP, sem
skills e sem memória automática, só com as ferramentas de leitura que cada tarefa precisa e um prompt de sistema
curto. Então você pode usar o Claude Code normalmente para outras coisas que o tkd não é afetado, e o tkd não lê nem
mexe nos seus projetos.

## Problemas comuns

| O que aparece | O que fazer |
|---|---|
| `'claude' não é reconhecido` | Feche e abra o terminal. Se continuar: `npm install -g @anthropic-ai/claude-code` |
| `Invalid API key` / pede login de novo | Rode `claude` e entre de novo |
| Muitas mensagens de limite (rate limit) | Plano Pro com Opus: troque o padrão para Sonnet e baixe `parallel` |
| Modelo não disponível | O seu plano não tem esse modelo: troque na tela **Agentes** |
