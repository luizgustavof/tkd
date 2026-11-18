Voce esta catalogando o banco de clipes do canal TikTok `{{slug}}`.

## Style card do canal
{{style_card}}

## Clipes a catalogar
Cada bloco traz o caminho do clipe, suas metricas e os frames extraidos dele.
Leia os PNGs com a ferramenta Read antes de decidir.

{{clips}}

## Tarefa
Para CADA clipe, diga o que ha nele e se serve ao canal.

## Formato (APENAS este JSON)
{
  "clips": [
    {
      "file": "<o mesmo caminho relativo que veio na lista>",
      "desc": "uma frase: quem/o que aparece e o que acontece",
      "tags": ["personagem", "acao", "clima", "plano (close/medio/aberto)"],
      "mood": "sombrio | melancolico | explosivo | calmo",
      "usable": true,
      "best_for": "hook | monologo | drop | fechamento",
      "quality": 0.0,
      "frames": [
        {"frame": "F0", "see": "o que esta nessa imagem, em poucas palavras", "chars": ["Nome Do Personagem"], "faces": ["Nome Do Personagem"]}
      ]
    }
  ]
}

`frames`: UM item por frame listado. Cada imagem tem um rotulo amarelo (F0, F1, ...) no canto: `frame` = o
rotulo que voce VE na imagem, e `see`/`chars`/`faces` descrevem ESSA imagem. Leia todas, inclusive as que parecem
iguais ou sao so um flash; nao pule nenhuma. `chars` = todos os
personagens visiveis naquele frame (nome canonico, ex. "Toji Fushiguro", "Satoru Gojo"; figura sem identificar =
"desconhecido"; ninguem = []). `faces` = de quem o rosto/boca aparece de forma reconhecivel (costas, nuca,
silhueta, maos e pes NAO contam).
`speakers` (so clipes com falas): [{"line": k, "who": "Nome"}] — quem FALA cada fala, pela cena em que ela soa (boca
mexendo) e pelo conteudo/contexto da serie; sem certeza = "desconhecido".

`quality` de 0 a 10: nitidez, enquadramento, ausencia de legenda queimada/marca d'agua de terceiros e encaixe no style card. `usable` false se tiver legenda queimada, logo de canal, tela preta, menu ou qualidade ruim.
