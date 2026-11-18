//! Bancada dos experimentos de caos: roda um cenario com o macaco ligado num processo filho e confere
//! se o app falhou limpo (sem processo orfao, banco integro, nenhum arquivo pela metade, nenhuma
//! publicacao pendurada). Relatorio em `out/chaos/<data>-<cenario>.md`.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Quem o cenario roda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Program {
    /// o proprio tkd (`current_exe`)
    Tkd,
    /// `evaluator/eval_video.py` no venv do avaliador
    Evaluator,
}

/// Como o cenario deve terminar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expect {
    /// tem que falhar (codigo != 0), e rapido
    Fail,
    /// tem que dar certo apesar do caos
    Ok,
    /// tanto faz; vale a limpeza e a mensagem
    Any,
}

/// Video de entrada ruim gerado na hora (ffmpeg na CPU) para os testes de dados ruins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BadInput {
    /// 3 s de video sem faixa de audio
    NoAudio,
    /// arquivo de 0 bytes com nome de mp4
    Empty,
    /// um frame so (33 ms), com audio
    OneFrame,
}

/// Um experimento: hipotese escrita antes, regra do macaco e o comando.
pub struct Scenario {
    pub name: &'static str,
    pub hypothesis: &'static str,
    /// `TKD_CHAOS` do filho ("" = macaco desligado; o ataque vem de fora: kill_named, kill_self, hold_*)
    pub chaos: &'static str,
    /// argumentos; `{channel}`, `{id}`, `{video}` (mp4 da biblioteca do --id), `{input}` e `{seed}` sao trocados
    pub args: &'static [&'static str],
    pub program: Program,
    pub needs_id: bool,
    /// teto do experimento inteiro
    pub limit: Duration,
    /// `TKD_CHAOS_LIMIT` do filho: encurta os tetos internos para um `hang` virar timeout em segundos
    pub inner_limit: Option<&'static str>,
    pub expect: Expect,
    /// mata o processo com esse nome quando ele aparecer na arvore do filho (depois do atraso)
    pub kill_named: Option<(&'static str, Duration)>,
    /// mata SO o filho (app/CLI fechado de supetao) depois desse tempo
    pub kill_self: Option<Duration>,
    /// segura uma escrita no state.db por esse tempo enquanto o filho roda
    pub hold_db: Option<Duration>,
    /// segura essa trava de arquivo (relativa a raiz; `{channel}` trocado) por esse tempo
    pub hold_file: Option<(&'static str, Duration)>,
    /// tem que demorar pelo menos isso (prova que esperou a trava)
    pub min_secs: Option<u64>,
    pub input: Option<BadInput>,
    /// depois roda o mesmo comando sem caos e ele tem que dar certo (a proxima tentativa funciona)
    pub then_ok: bool,
    /// ambiente extra do filho (mesmos marcadores dos args, mais `{root}`)
    pub env: &'static [(&'static str, &'static str)],
    /// servidor HTTP falso (`scripts/fake_http.py`, modo drop | stall) servindo o `{video}` em `{http}`
    pub server: Option<&'static str>,
    /// o ataque e a propria entrada (dados ruins): conta como atacado mesmo sem injecao
    pub bad_data: bool,
    /// `TEMP`/`TMP` do filho num disco minusculo de verdade (`TKD_CHAOS_DISK`, criado por scripts/chaos_disk.ps1)
    pub small_disk: bool,
    /// live falsa sai do ar aos `.0` s e volta `.1` s depois (arquivo de estado do streamlink falso)
    pub flap: Option<(u64, u64)>,
    /// congela a arvore inteira do filho aos `.0` s por `.1` s (PC dormindo)
    pub freeze: Option<(u64, u64)>,
}

const fn base(name: &'static str, hypothesis: &'static str, chaos: &'static str, args: &'static [&'static str]) -> Scenario {
    Scenario {
        name,
        hypothesis,
        chaos,
        args,
        program: Program::Tkd,
        needs_id: false,
        limit: Duration::from_secs(10 * 60),
        inner_limit: None,
        expect: Expect::Fail,
        kill_named: None,
        kill_self: None,
        hold_db: None,
        hold_file: None,
        min_secs: None,
        input: None,
        then_ok: false,
        env: &[],
        server: None,
        bad_data: false,
        small_disk: false,
        flap: None,
        freeze: None,
    }
}

/// Live falsa sem rede: streamlink de mentira e os 3 chats apontando para uma porta fechada (os cenarios de chat
/// trocam pelo chat falso). Antes os cenarios de reabrir/dormir conectavam no IRC real da Twitch.
const FAKE_LIVE: &[(&str, &str)] = &[
    ("TKD_STREAMLINK", r"{root}\scripts\fake_streamlink.cmd"),
    ("TKD_FAKE_SRC", "{video}"),
    ("TKD_TWITCH_IRC", "127.0.0.1:9"),
    ("TKD_YOUTUBE_BASE", "http://127.0.0.1:9"),
    ("TKD_KICK_BASE", "http://127.0.0.1:9"),
];

/// Live falsa: o streamlink de mentira transmite o mp4 da biblioteca em loop (sem rede).
const LIVE: &[&str] = &["live", "--url", "https://caos.invalid/live", "--minutes", "4", "--no-chat", "--threshold", "0.8"];

const RENDER: &[&str] = &["render", "--channel", "{channel}", "--id", "{id}"];
const PUBLISH_DRY: &[&str] = &["publish", "--channel", "{channel}", "--id", "{id}", "--dry-run"];
const IDEATE: &[&str] = &["ideate", "--channel", "{channel}"];
const EVAL: &[&str] = &["{video}", "--caption", "caos {seed}"];
/// Tudo que custa dinheiro ou rede fica travado: o run para no primeiro passo caro sem gastar nada.
const NO_COST: &str = "hang:claude,hang:yt-dlp,hang:curl,hang:node,hang:upload,hang:upload_yt";

pub const SCENARIOS: &[Scenario] = &[
    // ---------------------------------------------------------------- B. processos filhos
    Scenario {
        needs_id: true,
        limit: Duration::from_secs(15 * 60),
        ..base(
            "render-kill",
            "Matar o Remotion no meio do render: o comando falha, nenhum node/Chromium fica orfao e nenhum mp4 pela \
             metade fica com o nome final em out/renders/.",
            "kill:npx:1:25s",
            RENDER,
        )
    },
    Scenario {
        needs_id: true,
        limit: Duration::from_secs(15 * 60),
        ..base(
            "render-crash",
            "Matar SO o npx.cmd (netos vivos) no meio do render: node/Chromium caem junto e o tkd nao fica esperando.",
            "crash:npx:1:25s",
            RENDER,
        )
    },
    Scenario {
        needs_id: true,
        ..base(
            "publish-dry-kill",
            "Navegador morto no meio de um upload de TESTE: erro claro, Edge do perfil fechado, nenhuma reserva de \
             publicacao pendurada (dry-run nao reserva). ABRE O TIKTOK E O YOUTUBE.",
            "kill:upload:1:40s,kill:upload_yt:1:40s",
            PUBLISH_DRY,
        )
    },
    Scenario {
        needs_id: true,
        limit: Duration::from_secs(5 * 60),
        inner_limit: Some("20s"),
        ..base(
            "publish-dry-hang",
            "Script de upload travado: o teto do navegador corta (encurtado para 20 s), a trava do navegador fica \
             livre e nao sobra Edge. Nao abre navegador nenhum.",
            "hang:upload,hang:upload_yt",
            PUBLISH_DRY,
        )
    },
    Scenario {
        limit: Duration::from_secs(5 * 60),
        inner_limit: Some("15s"),
        ..base("agent-hang", "claude -p travado: o teto do agente corta (encurtado) e o passo falha com mensagem.", "hang:claude", IDEATE)
    },
    Scenario {
        limit: Duration::from_secs(5 * 60),
        ..base(
            "agent-truncate",
            "Resposta do claude -p cortada no meio (simulada, sem gastar token): o ideate falha com mensagem e nao \
             grava spec pela metade.",
            "truncate:claude:1",
            IDEATE,
        )
    },
    Scenario {
        limit: Duration::from_secs(5 * 60),
        ..base("agent-empty", "Resposta vazia do claude -p (simulada): falha clara, nada gravado.", "empty:claude:1", IDEATE)
    },
    Scenario {
        limit: Duration::from_secs(5 * 60),
        ..base("agent-ratelimit", "429 do Claude (simulado): o passo falha com mensagem clara, sem loop e sem gastar tokens.", "ratelimit:claude:1", IDEATE)
    },
    Scenario {
        program: Program::Evaluator,
        needs_id: true,
        limit: Duration::from_secs(15 * 60),
        kill_named: Some(("llama-server.exe", Duration::from_secs(9))),
        then_ok: true,
        ..base(
            "eval-llama-kill",
            "Matar o llama-server no meio da avaliacao: a avaliacao falha sem gravar nota parcial no cache, a trava \
             da placa e liberada e a proxima avaliacao sobe o servidor de novo e termina.",
            "",
            EVAL,
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Any,
        ..base(
            "speech-kill",
            "Matar o speech.py (whisper) no meio: os momentos saem sem fala (com aviso) ou falha claro; nenhum json de \
             fala fica largado no %TEMP%.",
            "kill:speech:1:8s",
            &["moments", "--file", "{video}", "--speech", "en", "--top", "3"],
        )
    },
    // ---------------------------------------------------------------- C. rede
    Scenario {
        inner_limit: Some("20s"),
        limit: Duration::from_secs(5 * 60),
        ..base(
            "youtube-block",
            "YouTube 429/'not a bot' no download: o yt-dlp para no primeiro bloqueio (sem trocar de client e \
             estender o bloqueio), a pausa e gravada (num arquivo de teste, o real fica intacto) e o comando falha \
             claro. Nao sai rede nenhuma (o bloqueio e injetado).",
            "block:yt-dlp:1,fail:curl:1,hang:claude",
            &["moments", "--url", "https://www.youtube.com/watch?v=k18gSPjWDM0", "--top", "3"],
        )
    },
    Scenario {
        needs_id: true,
        server: Some("drop"),
        inner_limit: Some("90s"),
        limit: Duration::from_secs(5 * 60),
        ..base(
            "net-drop",
            "Internet cai no meio de um download (servidor local manda 40% e sai do ar): o yt-dlp falha, o comando              falha claro e nenhum mp4/.part fica no %TEMP% para a proxima tentativa retomar. Nao sai na internet.",
            "fail:curl:1",
            &["moments", "--url", "{http}", "--top", "3"],
        )
    },
    Scenario {
        needs_id: true,
        server: Some("stall"),
        inner_limit: Some("30s"),
        limit: Duration::from_secs(5 * 60),
        ..base(
            "net-stall",
            "Rede presa no meio do download (conexao aberta, sem bytes): o teto do yt-dlp corta (encurtado para 30 s),              o comando falha claro e nada fica no %TEMP%.",
            "fail:curl:1",
            &["moments", "--url", "{http}", "--top", "3"],
        )
    },
    // ---------------------------------------------------------------- D. disco e estado
    Scenario {
        expect: Expect::Ok,
        hold_db: Some(Duration::from_secs(6)),
        min_secs: Some(5),
        limit: Duration::from_secs(2 * 60),
        ..base(
            "db-locked",
            "state.db travado por outro processo por 6 s: o tkd espera (busy_timeout) e segue, sem erro.",
            "",
            &["status"],
        )
    },
    Scenario {
        expect: Expect::Ok,
        hold_db: Some(Duration::from_secs(20)),
        min_secs: Some(19),
        limit: Duration::from_secs(2 * 60),
        ..base(
            "db-locked-long",
            "state.db travado por 20 s (OneDrive sincronizando): o tkd espera (ate 30 s) e segue, sem erro.",
            "",
            &["status"],
        )
    },
    Scenario {
        expect: Expect::Fail,
        hold_db: Some(Duration::from_secs(45)),
        limit: Duration::from_secs(2 * 60),
        ..base(
            "db-locked-forever",
            "state.db travado por 45 s (mais que a espera de 30 s): falha com mensagem que diz o que fazer, sem              corromper o banco.",
            "",
            &["status"],
        )
    },
    Scenario {
        needs_id: true,
        server: Some("ok"),
        small_disk: true,
        limit: Duration::from_secs(5 * 60),
        ..base(
            "disk-full-download",
            "Disco cheio no meio de um download (TEMP num disco de 12 MB, video de 18 MB): o comando falha dizendo que o              disco encheu e nao sobra nada pela metade no disco. Download de um servidor local.",
            "",
            &["moments", "--url", "{http}", "--top", "3"],
        )
    },
    Scenario {
        needs_id: true,
        small_disk: true,
        limit: Duration::from_secs(15 * 60),
        ..base(
            "disk-full-render",
            "Disco cheio no meio do render (TEMP do Remotion num disco de 12 MB): o render falha claro, o mp4 final              fica intacto e os quadros temporarios nao ficam no disco.",
            "",
            RENDER,
        )
    },
    // ---------------------------------------------------------------- E. app e PC
    Scenario {
        kill_self: Some(Duration::from_secs(25)),
        limit: Duration::from_secs(3 * 60),
        ..base(
            "app-killed-mid-run",
            "App/CLI fechado de supetao no meio de um video: os filhos morrem junto (Job Object), o run fica aberto \
             e a proxima abertura do app marca como interrompido. Nada caro roda (claude/rede/navegador travados).",
            NO_COST,
            &["run", "--channel", "{channel}", "--dry-run"],
        )
    },
    Scenario {
        hold_file: Some((".profiles/.browser.lock", Duration::from_secs(8))),
        min_secs: Some(7),
        limit: Duration::from_secs(3 * 60),
        needs_id: true,
        ..base(
            "browser-lock",
            "Outro processo do tkd (app) com o navegador: a CLI espera a vez em vez de abrir um segundo Edge. Os \
             scripts falham de proposito depois da espera (nada abre).",
            "fail:upload:1,fail:upload_yt:1",
            PUBLISH_DRY,
        )
    },
    Scenario {
        hold_file: Some(("out/.busy/{channel}.lock", Duration::from_secs(30))),
        limit: Duration::from_secs(60),
        ..base(
            "channel-busy",
            "Outro processo do tkd fazendo video no mesmo canal: o segundo recusa na hora ('ja tem um video sendo feito').",
            NO_COST,
            &["run", "--channel", "{channel}", "--dry-run"],
        )
    },
    // ---------------------------------------------------------------- F. live
    Scenario {
        needs_id: true,
        expect: Expect::Any,
        limit: Duration::from_secs(8 * 60),
        env: &[
            ("TKD_STREAMLINK", r"{root}\scripts\fake_streamlink.cmd"),
            ("TKD_FAKE_SRC", "{video}"),
            ("TKD_FAKE_DIE_AFTER", "100"),
        ],
        ..base(
            "live-drop",
            "A live cai no meio (streamlink sai aos 100 s): o live.py percebe, fecha a captura e sai limpo (o vigia do              tkd reabre se a live voltar), sem ffmpeg/streamlink orfao e sem corte duplicado.",
            "",
            LIVE,
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        limit: Duration::from_secs(8 * 60),
        env: &[
            ("TKD_STREAMLINK", r"{root}\scripts\fake_streamlink.cmd"),
            ("TKD_FAKE_SRC", "{video}"),
            ("TKD_FAKE_GARBAGE_AT", "70"),
        ],
        ..base(
            "live-garbage",
            "Lixo no meio do stream (segmento corrompido no buffer): a captura segue ate o fim, o segmento ruim nao              derruba a analise nem o corte, e a live termina normal no tempo pedido.",
            "",
            LIVE,
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        server: Some("irc"),
        limit: Duration::from_secs(8 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-chat-drop-twitch",
            "Chat da Twitch cai aos 75 s (IRC falso local sai do ar): a live segue, o erro do chat aparece no log, os              cortes continuam so por audio/movimento depois da queda e a live termina normal.",
            "",
            &["live", "--url", "https://twitch.tv/caoschat", "--minutes", "4", "--threshold", "0.8"],
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        server: Some("youtube"),
        limit: Duration::from_secs(8 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-chat-drop-youtube",
            "Chat do YouTube cai aos 75 s (live_chat falso local sai do ar): igual ao da Twitch. Nada sai na internet              (o orcamento do YouTube vai para o arquivo de teste).",
            "",
            &["live", "--url", "https://www.youtube.com/watch?v=caosCAOS123", "--minutes", "4", "--threshold", "0.8"],
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        flap: Some((150, 60)),
        limit: Duration::from_secs(12 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-reopen",
            "A live cai aos 150 s e volta 60 s depois, com o vigia do piloto ligado (1 conferencia por minuto): o vigia \
             reabre a captura em ate ~90 s depois da volta, nenhum clipe e sobrescrito ou duplicado e tudo fecha \
             limpo no fim. Sem rede (streamlink falso).",
            "",
            &["chaos", "watch-live", "--url", "https://twitch.tv/caosreopen", "--minutes", "9"],
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        freeze: Some((150, 180)),
        limit: Duration::from_secs(14 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-sleep",
            "PC dorme 3 min no meio da live (a arvore inteira do tkd congelada, como na hibernacao): ao voltar a \
             captura reinicia (pelo vigia ou pelo proprio live.py), volta a gravar e a cortar, e nada fica orfao.",
            "",
            &["chaos", "watch-live", "--url", "https://twitch.tv/caossleep", "--minutes", "11"],
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        server: Some("kick"),
        limit: Duration::from_secs(8 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-chat-drop-kick",
            "Chat da Kick cai aos 75 s (API falsa local sai do ar): igual ao da Twitch, cortes seguem por audio/movimento.",
            "",
            &["live", "--url", "https://kick.com/caoschat", "--minutes", "4", "--threshold", "0.8"],
        )
    },
    Scenario {
        needs_id: true,
        expect: Expect::Ok,
        server: Some("kick429"),
        limit: Duration::from_secs(8 * 60),
        env: FAKE_LIVE,
        ..base(
            "live-chat-429-kick",
            "Kick passa a responder 429 aos 75 s (servidor no ar, IP limitado): o chat para de consultar, a queda aparece \
             no log e os cortes seguem sem o chat (antes o 429 nao marcava erro e o chat mudo seguia na nota).",
            "",
            &["live", "--url", "https://kick.com/caoschat", "--minutes", "4", "--threshold", "0.8"],
        )
    },
    // ---------------------------------------------------------------- G. dados ruins
    Scenario {
        needs_id: true,
        bad_data: true,
        inner_limit: Some("15s"),
        limit: Duration::from_secs(2 * 60),
        ..base(
            "recreate-no-character",
            "Recriar com um personagem que nao tem nenhum clipe no banco: recusa na hora mandando rodar `footage`, sem              analisar a ref nem chamar o agente (claude travado de proposito: se chegar nele, estoura o teto).",
            "hang:claude",
            &["recreate", "--channel", "{channel}", "--ref", "{video}", "--character", "Zxqvy Wqprk", "--no-compare"],
        )
    },
    Scenario {
        expect: Expect::Any,
        input: Some(BadInput::NoAudio),
        limit: Duration::from_secs(3 * 60),
        ..base("bad-noaudio-moments", "Video sem audio no detector de momentos: falha clara ou segue sem os sinais de audio, rapido.", "", &["moments", "--file", "{input}", "--top", "3"])
    },
    Scenario {
        expect: Expect::Fail,
        input: Some(BadInput::Empty),
        limit: Duration::from_secs(3 * 60),
        ..base("bad-empty-moments", "Arquivo de 0 bytes no detector de momentos: falha clara logo no comeco.", "", &["moments", "--file", "{input}", "--top", "3"])
    },
    Scenario {
        expect: Expect::Fail,
        input: Some(BadInput::OneFrame),
        limit: Duration::from_secs(3 * 60),
        ..base("bad-oneframe-moments", "Video de 1 frame no detector de momentos: falha clara logo no comeco, sem resultado vazio gravado.", "", &["moments", "--file", "{input}", "--top", "3"])
    },
    Scenario {
        program: Program::Evaluator,
        expect: Expect::Fail,
        input: Some(BadInput::NoAudio),
        limit: Duration::from_secs(5 * 60),
        ..base("bad-noaudio-eval", "Video sem audio no avaliador: recusado logo no comeco, sem subir o modelo.", "", &["{input}", "--caption", "caos"])
    },
    Scenario {
        program: Program::Evaluator,
        expect: Expect::Fail,
        input: Some(BadInput::OneFrame),
        limit: Duration::from_secs(5 * 60),
        ..base("bad-oneframe-eval", "Video de 1 frame no avaliador: falha clara, sem nota inventada.", "", &["{input}", "--caption", "caos"])
    },
];

pub fn scenario(name: &str) -> Option<&'static Scenario> {
    SCENARIOS.iter().find(|s| s.name == name)
}

// ---------------------------------------------------------------- conferencias

#[derive(Debug, Clone, Deserialize)]
struct Proc {
    #[serde(rename = "ProcessId")]
    pid: u32,
    #[serde(rename = "ParentProcessId", default)]
    ppid: u32,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "CommandLine", default)]
    cmdline: Option<String>,
}

/// Processo que o tkd costuma abrir e que nao pode sobrar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Stray {
    pub pid: u32,
    pub name: String,
    pub cmdline: String,
}

const CHILD_NAMES: &[&str] = &[
    "node.exe",
    "ffmpeg.exe",
    "ffprobe.exe",
    "llama-server.exe",
    "python.exe",
    "pythonw.exe",
    "claude.exe",
    "msedge.exe",
    "chrome.exe",
    "chrome-headless-shell.exe",
    "streamlink.exe",
    "yt-dlp.exe",
    "curl.exe",
    "cmd.exe",
];
const OWNERS: &[&str] = &["tkd.exe", "tkd-app.exe"];

fn is_ours(p: &Proc, root: &str) -> bool {
    let name = p.name.to_lowercase();
    if !CHILD_NAMES.contains(&name.as_str()) {
        return false;
    }
    let c = p.cmdline.clone().unwrap_or_default().to_lowercase().replace('/', "\\");
    // `claude -p` do tkd (o claude interativo do usuario nao e nosso)
    if name == "claude.exe" {
        return c.contains("--strict-mcp-config") && c.contains("--disable-slash-commands");
    }
    c.contains(root) || c.contains("\\.profiles\\") || c.contains("remotion") || c.contains("llama-server")
}

/// Processos do tkd vivos sem um tkd/app acima deles na arvore (orfaos).
pub fn strays() -> Result<Vec<Stray>> {
    let procs = list_processes()?;
    let root = crate::config::root().to_string_lossy().to_lowercase().replace('/', "\\");
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let owned = |p: &Proc| {
        let mut cur = p.ppid;
        for _ in 0..32 {
            let Some(q) = by_pid.get(&cur) else { return false };
            if OWNERS.contains(&q.name.to_lowercase().as_str()) {
                return true;
            }
            if q.ppid == cur {
                return false;
            }
            cur = q.ppid;
        }
        false
    };
    Ok(procs
        .iter()
        .filter(|p| is_ours(p, &root) && !owned(p))
        .map(|p| Stray { pid: p.pid, name: p.name.clone(), cmdline: p.cmdline.clone().unwrap_or_default() })
        .collect())
}

fn list_processes() -> Result<Vec<Proc>> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-Command",
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-CimInstance Win32_Process | \
         Select-Object ProcessId,ParentProcessId,Name,CommandLine | ConvertTo-Json -Compress",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("listando processos")?;
    anyhow::ensure!(out.status.success(), "Get-CimInstance falhou: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str(text.trim()).context("lendo a lista de processos")?)
}

/// Arquivos pela metade / temporarios largados (relativos a raiz).
pub fn partial_files() -> BTreeSet<String> {
    let root = crate::config::root();
    let mut out = BTreeSet::new();
    for dir in ["out", "channels", "analysis", "evaluator\\bench"] {
        let base = root.join(dir);
        for e in walkdir::WalkDir::new(&base)
            .into_iter()
            .filter_entry(|e| !matches!(e.file_name().to_str(), Some("node_modules" | ".git" | "chaos")))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if is_partial(&n) {
                out.insert(e.path().strip_prefix(&root).unwrap_or(e.path()).to_string_lossy().to_string());
            }
        }
    }
    out
}

fn is_partial(name: &str) -> bool {
    name.ends_with(".tmp")
        || name.ends_with(".part")
        || name.ends_with(".part.mp4")
        || name.ends_with(".norm.mp4")
        || name.ends_with(".ytdl")
        || name.contains(".part-frag")
        || name.contains(".corrupt-")
}

/// `PRAGMA integrity_check` do state.db.
pub fn db_check() -> Result<String> {
    let conn = rusqlite::Connection::open(crate::state::db_path())?;
    conn.busy_timeout(Duration::from_secs(10))?;
    let v: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    Ok(v)
}

/// Fotografia do estado para comparar antes/depois.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub strays: Vec<Stray>,
    pub partial: BTreeSet<String>,
    pub db: String,
    pub unconfirmed: Vec<crate::state::Pending>,
}

pub fn snapshot() -> Result<Snapshot> {
    Ok(Snapshot {
        strays: strays()?,
        partial: partial_files(),
        db: db_check().unwrap_or_else(|e| format!("erro: {e:#}")),
        unconfirmed: crate::state::Store::open()?.unconfirmed_publishes(None)?,
    })
}

/// Texto do `tkd chaos check`.
pub fn describe(s: &Snapshot) -> String {
    let mut o = String::new();
    o.push_str(&format!("state.db: {}\n", s.db));
    o.push_str(&format!("processos orfaos: {}\n", s.strays.len()));
    for p in &s.strays {
        o.push_str(&format!("  {} {} {}\n", p.pid, p.name, p.cmdline.chars().take(160).collect::<String>()));
    }
    o.push_str(&format!("arquivos parciais/temporarios: {}\n", s.partial.len()));
    for f in &s.partial {
        o.push_str(&format!("  {f}\n"));
    }
    o.push_str(&format!("publicacoes sem confirmacao: {}\n", s.unconfirmed.len()));
    for p in &s.unconfirmed {
        o.push_str(&format!("  {} {} {} desde {} ({})\n", p.channel, p.platform, p.video_id, p.started_at, p.detail));
    }
    o
}

// ---------------------------------------------------------------- execucao

pub struct RunOpts<'a> {
    pub exe: PathBuf,
    pub channel: &'a str,
    pub id: Option<&'a str>,
    pub seed: Option<u64>,
}

/// Arquivo de estado do YouTube so dos experimentos (o real nao pode ser pausado por um teste).
fn chaos_yt_pace() -> PathBuf {
    crate::config::root().join("out").join("chaos").join("yt_pace.json")
}

/// Gera o video ruim (ffmpeg na CPU, sem -hwaccel).
async fn make_input(kind: BadInput) -> Result<PathBuf> {
    use crate::proc::TreeExt;
    let dir = crate::config::root().join("out").join("chaos").join("inputs");
    std::fs::create_dir_all(&dir)?;
    let (name, args): (&str, Vec<&str>) = match kind {
        BadInput::NoAudio => (
            "noaudio.mp4",
            vec!["-f", "lavfi", "-i", "testsrc=size=1080x1920:rate=30", "-t", "3", "-c:v", "libx264", "-pix_fmt", "yuv420p"],
        ),
        BadInput::OneFrame => (
            "oneframe.mp4",
            vec![
                "-f", "lavfi", "-i", "testsrc=size=1080x1920:rate=30", "-f", "lavfi", "-i", "sine=frequency=440",
                "-frames:v", "1", "-t", "0.033", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ],
        ),
        BadInput::Empty => {
            let p = dir.join("empty.mp4");
            std::fs::write(&p, b"")?;
            return Ok(p);
        }
    };
    let p = dir.join(name);
    if p.exists() {
        return Ok(p);
    }
    let out = tokio::process::Command::new(crate::media::ffmpeg_bin())
        .args(["-v", "error", "-y"])
        .args(&args)
        .arg(&p)
        .tree_output()
        .await
        .context("ffmpeg gerando a entrada ruim")?;
    anyhow::ensure!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    Ok(p)
}

fn command(sc: &Scenario, o: &RunOpts<'_>, args: &[String]) -> tokio::process::Command {
    let root = crate::config::root();
    let mut cmd = match sc.program {
        Program::Tkd => tokio::process::Command::new(&o.exe),
        Program::Evaluator => {
            let py = root.join("evaluator").join(".venv").join(if cfg!(windows) { "Scripts\\python.exe" } else { "bin/python" });
            let mut c = tokio::process::Command::new(py);
            c.args(["-I", "-X", "utf8"]).arg(root.join("evaluator").join("eval_video.py"));
            c.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
            c
        }
    };
    cmd.args(args)
        .current_dir(&root)
        .env("TKD_ROOT", &root)
        .env("TKD_YT_PACE_FILE", chaos_yt_pace());
    // sem Job Object proprio aqui: o que sobrar depois que o filho sair e orfao DELE (e o que se quer medir)
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    cmd
}

/// Pids da arvore de `root_pid` com esse nome.
fn named_in_tree(root_pid: u32, name: &str) -> Vec<u32> {
    let Ok(procs) = list_processes() else { return Vec::new() };
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    procs
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(name))
        .filter(|p| {
            let mut cur = p.ppid;
            for _ in 0..32 {
                if cur == root_pid {
                    return true;
                }
                match by_pid.get(&cur) {
                    Some(q) if q.ppid != cur => cur = q.ppid,
                    _ => return false,
                }
            }
            false
        })
        .map(|p| p.pid)
        .collect()
}

async fn taskkill(pid: u32) {
    let mut c = tokio::process::Command::new("taskkill");
    c.args(["/F", "/PID", &pid.to_string()]);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    let _ = c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await;
}

#[cfg(windows)]
#[link(name = "ntdll")]
extern "system" {
    fn NtSuspendProcess(h: windows_sys::Win32::Foundation::HANDLE) -> i32;
    fn NtResumeProcess(h: windows_sys::Win32::Foundation::HANDLE) -> i32;
}

/// Pids da arvore (raiz inclusa).
fn tree_pids(root_pid: u32) -> Vec<u32> {
    let Ok(procs) = list_processes() else { return vec![root_pid] };
    let mut out = vec![root_pid];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        let kids: Vec<u32> = procs.iter().filter(|q| q.ppid == p && q.pid != p && !out.contains(&q.pid)).map(|q| q.pid).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

/// Congela (true) ou descongela os processos.
#[cfg(windows)]
fn set_frozen(pids: &[u32], frozen: bool) -> usize {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SUSPEND_RESUME};
    let mut n = 0;
    for &pid in pids {
        unsafe {
            let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
            if h.is_null() {
                continue;
            }
            let r = if frozen { NtSuspendProcess(h) } else { NtResumeProcess(h) };
            if r == 0 {
                n += 1;
            }
            CloseHandle(h);
        }
    }
    n
}

#[cfg(not(windows))]
fn set_frozen(_: &[u32], _: bool) -> usize {
    0
}

fn live_state_file() -> PathBuf {
    crate::config::root().join("out").join("chaos").join("live_state.txt")
}

/// Ataques de fora do filho (o macaco dentro dele nao alcanca netos nem o proprio processo).
async fn outside_attacks(sc: &Scenario, pid: Option<u32>, log: std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let note = |m: String| {
        tracing::warn!("caos (lab): {m}");
        if let Ok(mut l) = log.lock() {
            l.push(m);
        }
    };
    let Some(pid) = pid else { return };
    if let Some((at, off)) = sc.flap {
        tokio::time::sleep(Duration::from_secs(at)).await;
        let _ = std::fs::write(live_state_file(), "off");
        note(format!("live saiu do ar em {}", chrono::Utc::now().to_rfc3339()));
        tokio::time::sleep(Duration::from_secs(off)).await;
        let _ = std::fs::write(live_state_file(), "on");
        note(format!("live voltou em {}", chrono::Utc::now().to_rfc3339()));
    }
    if let Some((at, dur)) = sc.freeze {
        tokio::time::sleep(Duration::from_secs(at)).await;
        let pids = tree_pids(pid);
        let n = set_frozen(&pids, true);
        note(format!("congelou {n} de {} processo(s) (PC dormiu) em {}", pids.len(), chrono::Local::now().format("%H:%M:%S")));
        tokio::time::sleep(Duration::from_secs(dur)).await;
        let n = set_frozen(&pids, false);
        note(format!("acordou {n} processo(s) em {}", chrono::Local::now().to_rfc3339()));
    }
    if let Some((name, after)) = sc.kill_named {
        // espera o processo aparecer na arvore, deixa ele trabalhar um pouco e mata so ele
        loop {
            let found = named_in_tree(pid, name);
            if let Some(&target) = found.first() {
                tokio::time::sleep(after).await;
                if named_in_tree(pid, name).contains(&target) {
                    taskkill(target).await;
                    note(format!("matou {name} (pid {target}) {:.0}s depois de aparecer", after.as_secs_f64()));
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    if let Some(after) = sc.kill_self {
        tokio::time::sleep(after).await;
        taskkill(pid).await;
        note(format!("matou o proprio filho (pid {pid}) depois de {}s, como quem fecha o app", after.as_secs()));
    }
}

/// Executa uma vez e devolve (saida, segundos).
async fn exec(mut cmd: tokio::process::Command, sc: &Scenario, attacks: bool, log: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> (std::io::Result<std::process::Output>, u64) {
    let t0 = std::time::Instant::now();
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (Err(e), 0),
    };
    let pid = child.id();
    let atk = if attacks { Some(tokio::spawn(outside_attacks_owned(sc.name, pid, log))) } else { None };
    let res = tokio::time::timeout(sc.limit, child.wait_with_output())
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "passou do teto"))
        .and_then(|r| r);
    if let Some(a) = atk {
        a.abort();
    }
    (res, t0.elapsed().as_secs())
}

async fn outside_attacks_owned(name: &'static str, pid: Option<u32>, log: std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    if let Some(sc) = scenario(name) {
        outside_attacks(sc, pid, log).await;
    }
}

fn live_dirs() -> BTreeSet<String> {
    std::fs::read_dir(crate::config::root().join("out").join("live"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc {
            if c.is_ascii_alphabetic() {
                esc = false;
            }
        } else if c == '\u{1b}' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Conferencias da live que cai e volta / do PC que dorme. Apaga as pastas de live do teste no fim.
fn live_checks(
    sc: &Scenario,
    res: &std::io::Result<std::process::Output>,
    outside: &[String],
    before: &BTreeSet<String>,
    checks: &mut Vec<(String, bool, String)>,
) {
    use chrono::{DateTime, NaiveTime, Utc};
    let full = match res {
        Ok(o) => strip_ansi(&format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(_) => String::new(),
    };
    let lines: Vec<&str> = full.lines().collect();
    let opened: Vec<DateTime<Utc>> = lines
        .iter()
        .filter(|l| l.contains("esta ao vivo: ligando o clipper"))
        .filter_map(|l| l.split_whitespace().next().and_then(|t| t.parse::<DateTime<Utc>>().ok()))
        .collect();
    let new_dirs: Vec<PathBuf> = live_dirs()
        .difference(before)
        .map(|d| crate::config::root().join("out").join("live").join(d))
        .collect();
    let clip_files: usize = new_dirs
        .iter()
        .map(|d| {
            std::fs::read_dir(d.join("clips"))
                .map(|r| {
                    r.flatten()
                        .filter(|e| {
                            let n = e.file_name().to_string_lossy().to_string();
                            n.starts_with("clip_") && n.ends_with(".json")
                        })
                        .count()
                })
                .unwrap_or(0)
        })
        .sum();
    let clip_logs = lines.iter().filter(|l| l.contains("  CLIP ")).count();
    checks.push((
        "nenhum clipe sobrescrito (CLIP no log = arquivos)".into(),
        clip_logs == clip_files && clip_files > 0,
        format!("{clip_logs} no log, {clip_files} arquivos em {} pasta(s)", new_dirs.len()),
    ));
    if sc.flap.is_some() {
        let back = outside
            .iter()
            .find_map(|l| l.strip_prefix("live voltou em "))
            .and_then(|t| t.parse::<DateTime<Utc>>().ok());
        let reopen = back.and_then(|b| opened.iter().find(|t| **t > b).map(|t| (*t - b).num_seconds()));
        checks.push((
            "o vigia reabriu depois da volta (<= 90 s)".into(),
            reopen.is_some_and(|s| s <= 90),
            match reopen {
                Some(s) => format!("{s}s depois da volta; {} abertura(s)", opened.len()),
                None => format!("nao reabriu; {} abertura(s)", opened.len()),
            },
        ));
        let mut seen = false;
        let after = lines
            .iter()
            .filter(|l| {
                if let (Some(b), Some(t)) = (back, l.split_whitespace().next().and_then(|t| t.parse::<DateTime<Utc>>().ok())) {
                    seen = t > b;
                }
                seen && l.contains("  CLIP ")
            })
            .count();
        checks.push(("cortou de novo depois da volta".into(), after > 0, format!("{after} corte(s)")));
    }
    if sc.freeze.is_some() {
        // live.py loga HH:MM:SS local; o acordar vem do lab
        let woke = outside
            .iter()
            .find_map(|l| l.strip_prefix("acordou ").and_then(|r| r.split(" em ").nth(1)))
            .and_then(|t| t.parse::<DateTime<chrono::Local>>().ok())
            .map(|t| t.time());
        let after_wake = |kw: &str| -> usize {
            let Some(w) = woke else { return 0 };
            let mut n = 0;
            let mut seen = false;
            for l in &lines {
                if let Some(t) = l.get(..8).and_then(|t| NaiveTime::parse_from_str(t, "%H:%M:%S").ok()) {
                    seen = t > w + chrono::Duration::seconds(20);
                }
                if seen && l.contains(kw) {
                    n += 1;
                }
            }
            n
        };
        let segs = after_wake("segmento ");
        let cuts = after_wake("  CLIP ");
        checks.push(("voltou a gravar depois de acordar".into(), segs >= 2, format!("{segs} segmento(s) gravados depois")));
        checks.push(("voltou a cortar depois de acordar".into(), cuts > 0, format!("{cuts} corte(s) depois")));
        let why: Vec<String> = lines
            .iter()
            .filter(|l| l.contains("captura parada") || l.contains("captura terminou") || l.contains("ligando o clipper"))
            .map(|l| l.chars().take(110).collect())
            .collect();
        checks.push(("(info) como a captura voltou".into(), true, why.join(" / ")));
    }
    for d in new_dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// MB livres no disco de `path` (PowerShell: sem dependencia nova).
fn free_mb(path: &str) -> Result<f64> {
    let letter = path.chars().next().filter(|c| c.is_ascii_alphabetic()).context(r"TKD_CHAOS_DISK precisa ser uma letra de disco (Q:\)")?;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-PSDrive {letter}).Free")])
        .output()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(s.parse::<f64>().with_context(|| format!("Get-PSDrive {letter}: {s:?}"))? / 1_048_576.0)
}

/// Arquivos (recursivo) em `dir`, fora as pastas do sistema.
fn files_under(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if n.eq_ignore_ascii_case("System Volume Information") || n.starts_with('$') {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.to_string_lossy().to_string());
            }
        }
    }
    out
}

fn open_runs(channel: &str) -> i64 {
    crate::state::Store::open()
        .and_then(|s| {
            Ok(s.db().query_row(
                "SELECT COUNT(*) FROM runs WHERE channel = ?1 AND step = 'run' AND status = 'start' AND ended_at IS NULL",
                rusqlite::params![channel],
                |r| r.get(0),
            )?)
        })
        .unwrap_or(-1)
}

fn spec_names() -> BTreeSet<String> {
    std::fs::read_dir(crate::ideate::specs_dir())
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default()
}

fn temp_tkd_files() -> BTreeSet<String> {
    std::fs::read_dir(std::env::temp_dir())
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("tkd-"))
                .collect()
        })
        .unwrap_or_default()
}

fn tail_of(res: &std::io::Result<std::process::Output>) -> (String, String) {
    match res {
        Ok(out) => {
            let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            let lines: Vec<&str> = all.lines().collect();
            (out.status.code().map(|c| c.to_string()).unwrap_or("?".into()), lines[lines.len().saturating_sub(30)..].join("\n"))
        }
        Err(e) => ("estourou o teto".to_string(), e.to_string()),
    }
}

/// Roda o cenario e devolve (passou de verdade: houve ataque e tudo conferiu, caminho do relatorio).
pub async fn run(sc: &Scenario, o: RunOpts<'_>) -> Result<(bool, PathBuf)> {
    let root = crate::config::root();
    // clone novo nao tem out/chaos: o estado da live falsa e as injecoes gravam ali (falhava com "caminho nao
    // encontrado" antes de rodar qualquer coisa)
    std::fs::create_dir_all(root.join("out").join("chaos")).context("criando out/chaos")?;
    // render sem --id: spec de bancada propria (40 s de texto), apagada no fim. O id nao comeca com o
    // slug do canal, entao `ideate::latest_id` (publish sem --id) nunca pega ela.
    let bench_id = format!("_chaos-bench-{}", o.channel);
    let _bench = if sc.args.first() == Some(&"render") && o.id.is_none() {
        write_bench_spec(o.channel, &bench_id)?;
        Some((
            crate::proc::RemoveOnDrop(crate::ideate::spec_path(&bench_id)),
            crate::proc::RemoveOnDrop(crate::render::mp4_path(&bench_id)),
        ))
    } else {
        None
    };
    let id = o.id.map(str::to_string).or_else(|| _bench.as_ref().map(|_| bench_id.clone()));
    anyhow::ensure!(!sc.needs_id || id.is_some(), "o cenario {} precisa de --id", sc.name);
    let seed = o.seed.unwrap_or_else(rand::random);
    let video = id
        .as_deref()
        .and_then(|i| crate::config::get_channel(o.channel).ok().map(|ch| ch.library_dir().join(i).join("out.mp4")))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if sc.args.contains(&"{video}") {
        anyhow::ensure!(Path::new(&video).exists(), "o cenario {} precisa de um --id da biblioteca (sem {video})", sc.name);
    }
    let input = match sc.input {
        Some(k) => make_input(k).await?.to_string_lossy().to_string(),
        None => String::new(),
    };
    // servidor HTTP falso: a "internet" do experimento, so em 127.0.0.1
    let outside: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let mut server = None;
    let mut http = String::new();
    let mut server_env: Vec<(String, String)> = Vec::new();
    if let Some(mode) = sc.server {
        use tokio::io::AsyncBufReadExt;
        anyhow::ensure!(Path::new(&video).exists(), "o cenario {} precisa de um --id da biblioteca", sc.name);
        let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let chat = matches!(mode, "irc" | "youtube" | "kick" | "kick429");
        let mut c = tokio::process::Command::new(crate::recreate::venv_bin("python"));
        c.args(["-I", "-X", "utf8"]);
        if chat {
            c.arg(root.join("scripts").join("fake_chat.py")).arg(port.to_string()).arg(mode).arg("75");
            server_env.push(match mode {
                "irc" => ("TKD_TWITCH_IRC".to_string(), format!("127.0.0.1:{port}")),
                "youtube" => ("TKD_YOUTUBE_BASE".to_string(), format!("http://127.0.0.1:{port}")),
                _ => ("TKD_KICK_BASE".to_string(), format!("http://127.0.0.1:{port}")),
            });
        } else {
            c.arg(root.join("scripts").join("fake_http.py")).arg(port.to_string()).arg(&video).arg(mode);
        }
        c.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = c.spawn().context("subindo scripts/fake_http.py")?;
        let mut lines = tokio::io::BufReader::new(child.stdout.take().context("stdout do servidor")?).lines();
        let first = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
            .context("servidor falso nao subiu")?;
        let log = outside.clone();
        log.lock().unwrap_or_else(|e| e.into_inner()).push(format!("servidor: {first}"));
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                log.lock().unwrap_or_else(|e| e.into_inner()).push(format!("servidor: {l}"));
            }
        });
        http = format!("http://127.0.0.1:{port}/v.mp4");
        server = Some(child);
    }
    // disco minusculo de verdade: so roda num disco pequeno (nunca enche o disco do sistema por engano)
    let small_disk = if sc.small_disk {
        let d = std::env::var("TKD_CHAOS_DISK").map_err(|_| {
            anyhow::anyhow!(
                r#"o cenario {} precisa de um disco minusculo: como ADMIN rode `powershell -ExecutionPolicy Bypass -File scripts\chaos_disk.ps1` e depois `$env:TKD_CHAOS_DISK = "Q:\"`"#,
                sc.name
            )
        })?;
        let free = free_mb(&d).with_context(|| format!("medindo {d}"))?;
        anyhow::ensure!(free < 100.0, "{d} tem {free:.0} MB livres: o experimento so roda num disco minusculo (< 100 MB)");
        let tmp = PathBuf::from(&d).join("tkd-tmp");
        std::fs::create_dir_all(&tmp)?;
        outside.lock().unwrap_or_else(|e| e.into_inner()).push(format!("TEMP do filho em {} ({free:.1} MB livres)", tmp.display()));
        Some((PathBuf::from(d), tmp))
    } else {
        None
    };
    let disk_before = small_disk.as_ref().map(|(d, _)| files_under(d)).unwrap_or_default();
    let args: Vec<String> = sc
        .args
        .iter()
        .map(|a| {
            a.replace("{channel}", o.channel)
                .replace("{id}", id.as_deref().unwrap_or_default())
                .replace("{video}", &video)
                .replace("{input}", &input)
                .replace("{http}", &http)
                .replace("{seed}", &seed.to_string())
        })
        .collect();
    let o = RunOpts { id: id.as_deref(), ..o };

    let before = snapshot()?;
    let specs_before = spec_names();
    let temp_before = temp_tkd_files();
    let runs_before = open_runs(o.channel);
    let render_out = o.id.map(crate::render::mp4_path);
    let render_mtime = render_out.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let real_pace = root.join("out").join(".yt_pace.json");
    let real_pace_before = std::fs::read(&real_pace).ok();
    let _ = std::fs::remove_file(chaos_yt_pace());
    let inj_path = root.join("out").join("chaos").join("injections.jsonl");
    let inj_from = std::fs::metadata(&inj_path).map(|m| m.len()).unwrap_or(0);

    // travas seguradas "por outro processo" (este) enquanto o filho roda
    let mut holders = Vec::new();
    if let Some(d) = sc.hold_db {
        let conn = rusqlite::Connection::open(crate::state::db_path())?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        outside.lock().unwrap_or_else(|e| e.into_inner()).push(format!("segurou escrita no state.db por {}s", d.as_secs()));
        holders.push(tokio::spawn(async move {
            tokio::time::sleep(d).await;
            let _ = conn.execute_batch("ROLLBACK");
            drop(conn);
        }));
    }
    if let Some((rel, d)) = sc.hold_file {
        let p = root.join(rel.replace("{channel}", o.channel));
        let lock = crate::proc::ExclusiveFile::try_acquire(&p).ok_or_else(|| anyhow::anyhow!("trava {} ja esta com alguem", p.display()))?;
        outside.lock().unwrap_or_else(|e| e.into_inner()).push(format!("segurou {rel} por {}s", d.as_secs()));
        holders.push(tokio::spawn(async move {
            tokio::time::sleep(d).await;
            drop(lock);
        }));
    }

    let label = match sc.program {
        Program::Tkd => "tkd",
        Program::Evaluator => "eval_video.py",
    };
    tracing::info!("caos: cenario {} (semente {seed}): {label} {}", sc.name, args.join(" "));
    let fill = |a: &str| {
        a.replace("{channel}", o.channel)
            .replace("{id}", o.id.unwrap_or_default())
            .replace("{video}", &video)
            .replace("{input}", &input)
            .replace("{seed}", &seed.to_string())
            .replace("{root}", &root.to_string_lossy())
    };
    let env: Vec<(String, String)> = sc.env.iter().map(|(k, v)| (k.to_string(), fill(v))).collect();
    let mut cmd = command(sc, &o, &args);
    cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    cmd.envs(server_env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let live_before: BTreeSet<String> = live_dirs();
    if sc.flap.is_some() || sc.freeze.is_some() {
        std::fs::write(live_state_file(), "on")?;
        cmd.env("TKD_FAKE_STATE", live_state_file());
    }
    if let Some((_, tmp)) = &small_disk {
        cmd.env("TEMP", tmp).env("TMP", tmp);
    }
    if !sc.chaos.is_empty() {
        cmd.env("TKD_CHAOS", sc.chaos).env("TKD_CHAOS_SEED", seed.to_string());
    }
    if let Some(l) = sc.inner_limit {
        cmd.env("TKD_CHAOS_LIMIT", l);
    }
    let (res, secs) = exec(cmd, sc, true, outside.clone()).await;
    for h in holders {
        let _ = h.await;
    }
    if let Some(mut s) = server {
        let _ = s.kill().await;
    }
    // o Job Object fecha na saida do filho; da um respiro para o Windows terminar de matar a arvore
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = snapshot()?;
    let (code, tail) = tail_of(&res);

    // a proxima tentativa (sem caos) tem que funcionar
    let retry = if sc.then_ok {
        let (r, s) = exec(command(sc, &o, &args), sc, false, outside.clone()).await;
        Some((r, s))
    } else {
        None
    };

    let injections = std::fs::read(&inj_path)
        .map(|b| String::from_utf8_lossy(&b[(inj_from as usize).min(b.len())..]).to_string())
        .unwrap_or_default();
    let outside_log = outside.lock().map(|l| l.clone()).unwrap_or_default();

    let mut checks: Vec<(String, bool, String)> = Vec::new();
    let new_strays: Vec<&Stray> = after.strays.iter().filter(|p| !before.strays.iter().any(|b| b.pid == p.pid)).collect();
    checks.push((
        "sem processo orfao".into(),
        new_strays.is_empty(),
        new_strays.iter().map(|p| format!("{} {} {}", p.pid, p.name, p.cmdline.chars().take(120).collect::<String>())).collect::<Vec<_>>().join("; "),
    ));
    checks.push(("state.db integro".into(), after.db == "ok", after.db.clone()));
    let new_partial: Vec<&String> = after.partial.difference(&before.partial).collect();
    checks.push((
        "nenhum arquivo pela metade".into(),
        new_partial.is_empty(),
        new_partial.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
    ));
    let temp_after = temp_tkd_files();
    let new_temp: Vec<&String> = temp_after.difference(&temp_before).collect();
    checks.push((
        "nada novo largado no %TEMP%".into(),
        new_temp.is_empty(),
        new_temp.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
    ));
    let new_unconf: Vec<_> =
        after.unconfirmed.iter().filter(|p| !before.unconfirmed.iter().any(|b| b.video_id == p.video_id && b.platform == p.platform)).collect();
    checks.push((
        "nenhuma publicacao pendurada".into(),
        new_unconf.is_empty(),
        new_unconf.iter().map(|p| format!("{} {} {}", p.channel, p.platform, p.video_id)).collect::<Vec<_>>().join(", "),
    ));
    let bad_specs: Vec<String> = spec_names()
        .difference(&specs_before)
        .filter(|n| {
            std::fs::read_to_string(crate::ideate::specs_dir().join(n.as_str()))
                .ok()
                .and_then(|t| serde_json::from_str::<crate::spec::Spec>(&t).ok())
                .is_none()
        })
        .cloned()
        .collect();
    checks.push(("nenhuma spec nova invalida".into(), bad_specs.is_empty(), bad_specs.join(", ")));
    if sc.flap.is_some() || sc.freeze.is_some() {
        live_checks(sc, &res, &outside_log, &live_before, &mut checks);
    }
    if matches!(sc.server, Some("irc" | "youtube" | "kick" | "kick429")) {
        let full = match &res {
            Ok(out) => format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
            Err(_) => String::new(),
        };
        let lines: Vec<&str> = full.lines().collect();
        let got_msgs = lines.iter().any(|l| {
            l.split("| chat ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0) > 0
        });
        let err_at = lines.iter().position(|l| l.contains("erro chat"));
        // o corte tem que vir da nota SEM chat (antes so valia corte de pico anterior a queda, ainda com chat)
        let dead_at = lines.iter().position(|l| l.contains("chat fora do ar"));
        let clip_after = dead_at.is_some_and(|i| lines[i..].iter().any(|l| l.contains("CLIP ")));
        checks.push(("chat chegou antes da queda".into(), got_msgs, String::new()));
        checks.push(("queda do chat aparece no log".into(), err_at.is_some(), String::new()));
        checks.push(("cortes continuam depois da queda (nota sem o chat)".into(), clip_after, String::new()));
    }
    if let Some((d, _)) = &small_disk {
        let left: Vec<String> = files_under(d).difference(&disk_before).cloned().collect();
        checks.push(("nada largado no disco cheio".into(), left.is_empty(), left.join(", ")));
        let t = tail.to_lowercase();
        let says = ["space", "espaco", "espaço", "enospc", "disco", "disk"].iter().any(|k| t.contains(k));
        checks.push(("a mensagem diz que o disco encheu".into(), says, String::new()));
    }
    // todo cenario: o orcamento/pausa do YouTube dos canais de verdade nunca muda por causa de um teste
    checks.push(("estado real do YouTube intacto".into(), std::fs::read(&real_pace).ok() == real_pace_before, String::new()));
    checks.push(("terminou dentro do teto".into(), res.is_ok(), format!("{secs}s de {}s", sc.limit.as_secs())));
    let success = matches!(&res, Ok(out) if out.status.success());
    let killed_self = sc.kill_self.is_some();
    if !killed_self {
        let (ok, want) = match sc.expect {
            Expect::Fail => (res.is_ok() && !success, "falhar"),
            Expect::Ok => (success, "dar certo"),
            Expect::Any => (true, "qualquer saida limpa"),
        };
        checks.push((format!("terminou como esperado ({want})"), ok, format!("codigo {code}")));
    }
    // erro de uso (argumento errado no cenario) nao e falha limpa: o experimento nem rodou
    let usage = sc.program == Program::Tkd && tail.contains("Usage: tkd");
    checks.push(("comando do cenario valido".into(), !usage, if usage { "erro de argumento".into() } else { String::new() }));
    if let Some(min) = sc.min_secs {
        checks.push(("esperou a trava".into(), secs >= min, format!("{secs}s (minimo {min}s)")));
    }
    if sc.args.first() == Some(&"render") {
        let changed = render_out.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()) != render_mtime;
        checks.push((
            "mp4 final intacto (so troca se o render terminou)".into(),
            !(!success && changed),
            if changed { "mp4 final mudou".into() } else { "mp4 final igual".into() },
        ));
    }
    if killed_self {
        let open_after = open_runs(o.channel);
        let opened = open_after > runs_before;
        let closed = crate::state::Store::open().and_then(|s| s.close_open_runs()).unwrap_or(0);
        checks.push((
            "run ficou aberto e a proxima abertura marca como interrompido".into(),
            opened && closed > 0 && open_runs(o.channel) == 0,
            format!("abertos antes {runs_before}, depois do kill {open_after}, fechados {closed}"),
        ));
    }
    if sc.name == "youtube-block" {
        let calls = injections.matches("not a bot").count();
        let paused = std::fs::read_to_string(chaos_yt_pace())
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["paused_until"].as_u64())
            .unwrap_or(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        checks.push(("parou no primeiro bloqueio (sem trocar de client)".into(), calls == 1, format!("{calls} chamada(s) bloqueada(s)")));
        checks.push(("pausa gravada no arquivo de teste".into(), paused > now, format!("pausa ate {paused}")));
    }
    if let Some((r, s)) = &retry {
        let ok = matches!(r, Ok(out) if out.status.success());
        let (c, t) = tail_of(r);
        let last = t.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect::<String>();
        checks.push(("a proxima tentativa (sem caos) funciona".into(), ok, format!("codigo {c}, {s}s: {}", last.replace('|', "/"))));
    }
    if sc.name == "eval-llama-kill" {
        // nota parcial nao pode ter ido para o cache: a rodada sem caos tem que ter avaliado de verdade
        let cached = retry.as_ref().map(|(r, _)| tail_of(r).1.contains("\"cached\": true")).unwrap_or(false);
        checks.push(("cache sem nota parcial".into(), !cached, if cached { "a 2a rodada leu do cache".into() } else { String::new() }));
    }

    // houve ataque? (macaco dentro do filho, ou de fora pelo lab)
    let wants_kill = crate::chaos::parse(sc.chaos)
        .map(|r| r.iter().any(|r| matches!(r.kind, crate::chaos::Kind::Kill | crate::chaos::Kind::Crash)))
        .unwrap_or(false);
    let attacked = if sc.flap.is_some() {
        outside_log.iter().any(|l| l.starts_with("live voltou"))
    } else if sc.freeze.is_some() {
        outside_log.iter().any(|l| l.starts_with("acordou"))
    } else if sc.small_disk {
        true
    } else if sc.server == Some("ok") {
        true
    } else if sc.server.is_some() {
        outside_log.iter().any(|l| l.contains("derrubou") || l.contains("travou") || l.contains("caiu"))
    } else if sc.input.is_some() || !sc.env.is_empty() || sc.bad_data {
        true
    } else if !outside_log.is_empty() {
        true
    } else if wants_kill {
        injections.contains("\"kind\":\"kill\"")
    } else {
        !injections.trim().is_empty()
    };
    let attacked = attacked && (sc.kill_named.is_none() || outside_log.iter().any(|l| l.starts_with("matou")));
    let passed = checks.iter().all(|c| c.1);
    let verdict = match (attacked, passed) {
        (false, _) => "INCONCLUSIVO (o ataque nao aconteceu: o alvo nao rodou ou terminou/falhou antes)",
        (true, true) => "PASSOU",
        (true, false) => "FALHOU",
    };

    let mut md = String::new();
    md.push_str(&format!("# Caos: {}\n\n", sc.name));
    md.push_str(&format!("- **Resultado:** {verdict}\n"));
    md.push_str(&format!("- **Quando:** {}\n", chrono::Local::now().format("%d/%m/%Y %H:%M")));
    md.push_str(&format!("- **Hipotese:** {}\n", sc.hypothesis));
    md.push_str(&format!(
        "- **Regra:** `TKD_CHAOS={}` `TKD_CHAOS_SEED={seed}`{}\n",
        sc.chaos,
        sc.inner_limit.map(|l| format!(" `TKD_CHAOS_LIMIT={l}`")).unwrap_or_default()
    ));
    md.push_str(&format!("- **Comando:** `{label} {}`\n", args.join(" ")));
    md.push_str(&format!("- **Saida:** codigo {code}, {secs}s\n\n"));
    md.push_str("## Conferencias\n\n| | Conferencia | Detalhe |\n|---|---|---|\n");
    for (name, ok, detail) in &checks {
        md.push_str(&format!("| {} | {name} | {} |\n", if *ok { "ok" } else { "FALHOU" }, detail.replace('|', "/")));
    }
    md.push_str(&format!("\n## Ataques\n\n```\n{}\n{}\n```\n", outside_log.join("\n"), injections.trim()));
    md.push_str(&format!("\n## Fim da saida\n\n```\n{}\n```\n", tail.trim()));

    let dir = root.join("out").join("chaos");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}-{}.md", chrono::Local::now().format("%Y%m%d-%H%M%S"), sc.name));
    std::fs::write(&path, md)?;
    Ok((passed && attacked, path))
}

/// Spec so de texto, longa o bastante (40 s) para o macaco matar o render no meio.
fn write_bench_spec(channel: &str, id: &str) -> Result<()> {
    let scenes: Vec<serde_json::Value> = (0..5)
        .map(|i| serde_json::json!({ "startS": i as f64 * 8.0, "durationS": 8.0, "kind": "text", "text": format!("teste de caos {i}") }))
        .collect();
    let v = serde_json::json!({
        "id": id, "channel": channel, "title": "caos", "durationS": 40.0, "aspect": "9:16", "fps": 30.0,
        "palette": { "bg": "#000000", "fg": "#ffffff", "accent": "#ff2e63" },
        "typography": { "family": "Anton", "weight": 400, "case": "upper", "sizeVh": 6.0 },
        "treatment": { "grain": 0.0, "vignette": 0.0, "zoom": "none" },
        "scenes": scenes, "caption": "teste de caos", "hashtags": [],
    });
    let spec: crate::spec::Spec = serde_json::from_value(v).context("spec de bancada")?;
    std::fs::create_dir_all(crate::ideate::specs_dir())?;
    std::fs::write(crate::ideate::spec_path(id), serde_json::to_string_pretty(&spec)?)?;
    Ok(())
}

/// Caminho relativo legivel (para o terminal).
pub fn rel(p: &Path) -> String {
    p.strip_prefix(crate::config::root()).unwrap_or(p).to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_partials() {
        for n in ["clips.json.123.0.tmp", "x.part", "abc.part.mp4", "v.mp4.ytdl", "v.f137.mp4.part-frag12", "clips.json.corrupt-20261118"] {
            assert!(is_partial(n), "{n}");
        }
        for n in ["out.mp4", "clips.json", "clips.json.bak", "partner.mp4"] {
            assert!(!is_partial(n), "{n}");
        }
    }

    #[test]
    fn counts_only_tkd_processes() {
        let root = "c:\\tk";
        let p = |name: &str, c: &str| Proc { pid: 1, ppid: 0, name: name.into(), cmdline: Some(c.into()) };
        assert!(is_ours(&p("node.exe", "node C:\\tk\\browser\\upload.mjs"), root));
        assert!(is_ours(&p("msedge.exe", "msedge --user-data-dir=C:\\x\\.profiles\\canal"), root));
        assert!(is_ours(&p("claude.exe", "claude -p --strict-mcp-config --disable-slash-commands"), root));
        assert!(!is_ours(&p("claude.exe", "claude"), root));
        assert!(!is_ours(&p("node.exe", "node C:\\outro\\server.js"), root));
        assert!(!is_ours(&p("explorer.exe", "C:\\tk"), root));
    }

    #[test]
    fn valid_scenarios() {
        for s in SCENARIOS {
            assert!(crate::chaos::parse(s.chaos).is_ok(), "{}", s.name);
            let wants_id = s.args.iter().chain(s.env.iter().map(|(_, v)| v)).any(|a| a.contains("{id}") || a.contains("{video}"));
            assert!(s.needs_id == (wants_id || s.server.is_some()), "{}", s.name);
        }
    }
}
