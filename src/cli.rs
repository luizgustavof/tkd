use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tkd", about = "Fabrica de canais TikTok: estuda, gera, renderiza, publica")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Mostra o estado de cada canal (refs, biblioteca, ultimo post)
    Status,

    /// Confere ferramentas e configuracao da primeira execucao (ffmpeg, node, claude, yt-dlp, venv, Edge)
    Doctor,

    /// Consumo de tokens dos agentes (importa o historico do ~/.claude na primeira vez)
    Usage {
        /// Ultimos N dias (0 = tudo)
        #[arg(long, default_value_t = 0)]
        days: i64,
    },

    /// Abre o navegador no perfil do canal para login manual (sessao fica em .profiles/<canal>/)
    Login {
        /// Canal a vincular; sem isso, percorre todos os canais ainda sem conta
        #[arg(long)]
        channel: Option<String>,
        /// Minutos de espera pelo login em cada canal
        #[arg(long, default_value_t = 20)]
        timeout_min: u64,
    },

    /// Sai de uma plataforma no perfil do canal (apaga so os cookies dela; a outra continua logada)
    Logout {
        #[arg(long)]
        channel: String,
        /// tiktok | youtube
        #[arg(long)]
        platform: String,
    },

    /// Login de uma conta Google DESCARTAVEL para os cookies do yt-dlp (perfil .profiles/_youtube/)
    YtLogin {
        /// Minutos de espera pelo login
        #[arg(long, default_value_t = 15)]
        timeout_min: u64,
    },

    /// Lista qual conta esta vinculada a cada canal
    Accounts {
        /// Abre o navegador para conferir a sessao e gravar o @handle detectado
        #[arg(long)]
        check: bool,
        #[arg(long)]
        channel: Option<String>,
    },

    /// Varre os canais de referencia, baixa o que e novo e extrai metricas
    Scout {
        #[arg(long)]
        channel: Option<String>,
        /// Maximo de videos novos por canal nesta passada
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },

    /// Busca footage no YouTube, recorta em planos e cataloga no banco do canal
    Footage {
        #[arg(long)]
        channel: String,
        /// O que procurar; sem isso, o agente decide pelo style card
        #[arg(long)]
        topic: Option<String>,
        /// Quantas buscas diferentes fazer
        #[arg(long, default_value_t = 3)]
        queries: usize,
        /// Quantos planos recortar de cada video baixado
        #[arg(long, default_value_t = 6)]
        per_query: usize,
        /// Modo falas: busca cenas com a voz do personagem e recorta frases inteiras com audio.
        /// Valor = idioma da fala: en | ja | pt | auto
        #[arg(long)]
        speech: Option<String>,
        /// Personagem unico do edit (todas as buscas nele)
        #[arg(long)]
        character: Option<String>,
    },

    /// Cataloga o banco de clipes (descricao + quem aparece em cada frame). --refresh refaz os clipes antigos
    /// que ainda nao tem o catalogo por frame
    Catalog {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        refresh: bool,
    },

    /// Mostra os momentos fortes de um video (audio, movimento, fala, heatmap e comentarios do YouTube), sem recortar
    Moments {
        /// Video local; sem isso, baixa --url
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        /// Video do YouTube: traz heatmap + comentarios com timestamp
        #[arg(long)]
        url: Option<String>,
        /// Usa perfil/lexico/pesos do [moments] deste canal
        #[arg(long)]
        channel: Option<String>,
        /// fight | speech (sobrescreve o do canal)
        #[arg(long)]
        profile: Option<String>,
        /// Transcreve a fala (en | ja | pt | auto) para os sinais de fala/lexico
        #[arg(long)]
        speech: Option<String>,
        #[arg(long, default_value_t = 8)]
        top: usize,
    },

    /// Assiste uma live (Twitch, Kick, YouTube), le o chat sem login e corta os momentos fortes em 9:16
    Live {
        /// ex.: https://twitch.tv/canal | https://kick.com/canal | https://youtube.com/watch?v=...
        #[arg(long)]
        url: String,
        /// Minutos de captura (0 = ate a live acabar ou Ctrl+C)
        #[arg(long, default_value_t = 0.0)]
        minutes: f64,
        /// Nota minima do momento para cortar (mais alto = menos clipes, mais fortes)
        #[arg(long, default_value_t = 1.5)]
        threshold: f64,
        #[arg(long, default_value_t = 20.0)]
        min: f64,
        /// Duracao maxima do clipe (s); o tamanho real acompanha quanto a reacao dura
        #[arg(long, default_value_t = 60.0)]
        max: f64,
        /// Idioma da fala para o whisper (pt | en | ja | auto)
        #[arg(long, default_value = "auto")]
        lang: String,
        /// Queima a legenda do whisper no 9:16 (padrao: sem; o .srt sai sempre)
        #[arg(long)]
        captions: bool,
        /// Ignora o chat (so audio e movimento)
        #[arg(long)]
        no_chat: bool,
    },

    /// Baixa uma trilha do YouTube e mede a grade de batidas dela
    Music {
        #[arg(long)]
        channel: String,
        /// Busca no YouTube (ex: "phonk slowed beat instrumental")
        #[arg(long)]
        query: String,
    },

    /// Refaz a analise completa dos mp4 ja baixados (sem baixar nada)
    Analyze {
        #[arg(long)]
        channel: Option<String>,
    },

    /// Destila as referencias em style-card.md + preset.json
    Study {
        #[arg(long)]
        channel: Option<String>,
        /// Reescreve o style-card mesmo sem referencia nova
        #[arg(long)]
        force: bool,
    },

    /// Gera a spec de um video novo
    Ideate {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        theme: Option<String>,
    },

    /// Renderiza uma spec com Remotion
    Render {
        #[arg(long)]
        channel: String,
        /// id da spec; padrao = a mais recente do canal
        #[arg(long)]
        id: Option<String>,
    },

    /// Avalia o mp4 e promove para a biblioteca do canal se passar
    Review {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        id: Option<String>,
    },

    /// Publica um video aprovado no TikTok
    Publish {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        id: Option<String>,
        /// Sobe tudo mas nao clica em Post
        #[arg(long)]
        dry_run: bool,
        /// Rede da publicacao sem confirmacao a resolver (tiktok | youtube)
        #[arg(long)]
        platform: Option<String>,
        /// Publicacao sem confirmacao que SAIU: grava como postada com essa URL (nao reenvia)
        #[arg(long, requires = "platform")]
        mark_posted: Option<String>,
        /// Publicacao sem confirmacao que NAO saiu: libera para enviar de novo
        #[arg(long, requires = "platform", conflicts_with = "mark_posted")]
        retry_unconfirmed: bool,
        /// Post de teste: YouTube nao listado, TikTok "Somente voce" (aborta se nao conseguir marcar)
        #[arg(long)]
        test_post: bool,
    },

    /// Experimentos de caos (docs/CHAOS.md): conferir o estado ou rodar um cenario com o macaco ligado
    Chaos {
        #[command(subcommand)]
        cmd: ChaosCmd,
    },

    /// Pipeline completo de um canal: ideate -> render -> review -> publish
    Run {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        theme: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Post de teste: YouTube nao listado, TikTok "Somente voce"
        #[arg(long)]
        test_post: bool,
    },

    /// Recria a estrutura/acabamento de uma referencia com roteiro novo, footage do banco e voz ElevenLabs
    Recreate {
        #[arg(long)]
        channel: String,
        /// id da ref (channels/<canal>/refs/<id>.mp4) ou caminho de um mp4
        #[arg(long = "ref")]
        reference: String,
        /// Roteiro novo (o que a voz vai dizer)
        #[arg(long)]
        script: Option<String>,
        /// Arquivo com o roteiro
        #[arg(long)]
        script_file: Option<std::path::PathBuf>,
        /// Recria so os primeiros N segundos da ref
        #[arg(long)]
        end: Option<f64>,
        /// Voz ElevenLabs (padrao: $TKD_VOICE_ID ou Brian)
        #[arg(long)]
        voice_id: Option<String>,
        /// Pula o agente e usa um plano pronto ({plan, voice} ou so o plan)
        #[arg(long)]
        plan: Option<std::path::PathBuf>,
        /// Nao gera a comparacao ref x remake
        #[arg(long)]
        no_compare: bool,
        /// Voz: auto (falas reais do banco se houver, senao TTS) | clip | tts | none
        #[arg(long, default_value = "auto")]
        voice: String,
        /// Idioma do edit (legendas): en | pt
        #[arg(long, default_value = "en")]
        lang: String,
        /// Personagem do edit: so clipes e falas dele
        #[arg(long)]
        character: Option<String>,
        /// Movimento: high | medium | low | none (padrao: o agente escolhe pelo guia do canal)
        #[arg(long)]
        motion: Option<String>,
    },

    /// Loop autonomo: scout + study + pipeline dentro da janela de post
    Watch {
        /// Roda um ciclo e sai (uso com Task Scheduler)
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 6)]
        every_hours: u64,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ChaosCmd {
    /// Processos orfaos, state.db, arquivos pela metade e publicacoes sem confirmacao
    Check,
    /// Lista os cenarios
    List,
    /// Roda um cenario num tkd filho com TKD_CHAOS e grava o relatorio em out/chaos/
    Run {
        #[arg(long)]
        scenario: String,
        #[arg(long)]
        channel: String,
        #[arg(long)]
        id: Option<String>,
        /// Semente fixa (repete um experimento que achou bug)
        #[arg(long)]
        seed: Option<u64>,
    },
    /// So o vigia de live de uma URL (sem o resto do piloto), por N minutos. Usado pelos cenarios de live
    WatchLive {
        #[arg(long)]
        url: String,
        #[arg(long, default_value_t = 8.0)]
        minutes: f64,
        /// segundos entre conferencias (o piloto usa 60)
        #[arg(long, default_value_t = 60)]
        tick: u64,
        /// jogos aceitos (como o `games` do canal); repetir para varios
        #[arg(long = "game")]
        games: Vec<String>,
    },
}
