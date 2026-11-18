Voce escolhe o MELHOR corte de live para o canal TikTok/Shorts `{{slug}}`, entre {{n}} candidatos (indices 0 a {{max_i}}).

Canal (TEMA OBRIGATORIO): {{niche}}
Persona: {{persona}}

{{clips}}

## Observacoes do dono do canal
{{notes}}

## Regra 1: tema (eliminatoria)
O canal so posta o tema acima. Corte de outro jogo/assunto e `no_tema: false`, mesmo que seja otimo.
Confira pelos quadros (o que aparece na tela) e pela fala. Na duvida se e do tema, `no_tema: false`.
Use a "categoria da live" de cada corte: ela diz qual jogo a plataforma registrou no momento. Jogo parecido
nao vale (ex.: o canal e de GTA VI e a categoria e "Grand Theft Auto V" = `no_tema: false`, mesmo que os
quadros parecam GTA). Categoria "desconhecida" (YouTube) = decida pelos quadros e pela fala.

## Regra 2: qualidade (corte de live NAO e edit: nao cobre ritmo de edicao, musica ou efeitos)
1. Momento: engracado, tenso, absurdo, emocionante ou jogada marcante? O chat confirma?
2. Contexto: quem nunca viu a live entende em 2-3 s? Se precisa de contexto, a legenda resolve?
3. Desfecho: termina na reacao/punchline, sem cortar no meio nem enrolar depois?
4. Gancho: os 2 primeiros segundos prendem? Se nao, sugira comecar mais tarde (`start`).
5. Seguranca: nada de exposicao de terceiros, conteudo +18, odio ou dado pessoal na tela.

## Escolha
Avalie TODOS. `escolhido` = indice do melhor que e do tema e tem score >= 7; -1 se nenhum serve.
Titulo, legenda, hashtags e recorte sao do ESCOLHIDO.

## Formato (APENAS este JSON)
{
  "avaliacoes": [{"i": 0, "no_tema": true, "score": 0.0, "motivo": "frase curta"}],
  "escolhido": 0,
  "problems": ["problema concreto do escolhido"],
  "best_part": "o que funciona no escolhido",
  "title": "titulo curto em caixa alta, ate 6 palavras",
  "caption": "legenda curta no tom da persona, que gere comentario (sem hashtags, sem o credito: ele entra sozinho)",
  "hashtags": ["#streamer", "#jogo_ou_tema"],
  "start": null,
  "end": null
}

`score` 0-10 com uma casa. `start`/`end` em segundos DENTRO do corte escolhido (null = inteiro; minimo 12 s).
`hashtags`: 2 a 4 especificas; nao repita as fixas do canal: {{hashtags}}
