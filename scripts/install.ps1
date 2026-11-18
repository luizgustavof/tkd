# Instalador do tkd para Windows 10/11.
# Uso facil: duplo clique em instalar.bat (na raiz do projeto).
# Pelo terminal: powershell -ExecutionPolicy Bypass -File scripts\install.ps1 [-DryRun] [-SkipBuild] [-NoShortcut]
#
# Pode rodar de novo quantas vezes quiser: o que ja esta instalado e pulado.
# Instala com winget (vem no Windows 11 e no 10 atualizado). Nao mexe em nada fora do projeto alem dos programas
# listados abaixo. Tudo roda na CPU: nada de driver de GPU.

param(
    [switch]$DryRun,     # so mostra o que faria
    [switch]$SkipBuild,  # nao compila o app (ex.: so atualizar dependencias)
    [switch]$NoShortcut  # nao cria atalho na area de trabalho
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$script:Failed = @()

function Write-Step($n, $text) { Write-Host ""; Write-Host "[$n] $text" -ForegroundColor Cyan }
function Write-Ok($text) { Write-Host "    ok: $text" -ForegroundColor Green }
function Write-Todo($text) { Write-Host "    -> $text" -ForegroundColor Yellow }
function Write-Bad($text) { Write-Host "    ERRO: $text" -ForegroundColor Red; $script:Failed += $text }

# O PATH muda depois de cada instalacao do winget; recarrega sem precisar abrir outro terminal.
function Update-SessionPath {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machine;$user"
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path $cargo) -and ($env:Path -notlike "*$cargo*")) { $env:Path = "$cargo;$env:Path" }
}

function Test-Command($name) { return [bool](Get-Command $name -ErrorAction SilentlyContinue) }

function Invoke-Native($what, [scriptblock]$block) {
    if ($DryRun) { Write-Todo "(simulacao) $what"; return $true }
    Write-Todo $what
    & $block | Out-Host
    if ($LASTEXITCODE -ne 0) { Write-Bad "$what (codigo $LASTEXITCODE)"; return $false }
    return $true
}

function Install-Winget($id, $label, [string[]]$extra = @()) {
    $wargs = @('install', '--id', $id, '-e', '--silent', '--accept-package-agreements', '--accept-source-agreements') + $extra
    $ok = Invoke-Native "instalando $label (winget $id)" { winget @wargs }
    Update-SessionPath
    return $ok
}

Write-Host "Instalador do tkd" -ForegroundColor White
Write-Host "Pasta: $Root"
if ($DryRun) { Write-Host "Modo simulacao: nada sera instalado." -ForegroundColor Yellow }
Update-SessionPath

# ------------------------------------------------------------------ 1. winget
Write-Step 1 "Gerenciador de pacotes (winget)"
if (-not (Test-Command 'winget')) {
    Write-Bad "winget nao encontrado. Instale o 'Instalador de Aplicativo' pela Microsoft Store e rode de novo."
    exit 1
}
Write-Ok "winget"

# ------------------------------------------------------------------ 2. programas
Write-Step 2 "Programas (so instala o que falta)"
$tools = @(
    @{ Cmd = 'git';    Id = 'Git.Git';             Label = 'Git (e o Git Bash)' },
    @{ Cmd = 'node';   Id = 'OpenJS.NodeJS.LTS';   Label = 'Node.js LTS' },
    @{ Cmd = 'ffmpeg'; Id = 'Gyan.FFmpeg';         Label = 'FFmpeg' },
    @{ Cmd = 'py';     Id = 'Python.Python.3.12';  Label = 'Python 3.12' },
    @{ Cmd = 'rustup'; Id = 'Rustlang.Rustup';     Label = 'Rust' }
)
foreach ($t in $tools) {
    if (Test-Command $t.Cmd) { Write-Ok $t.Label } else { [void](Install-Winget $t.Id $t.Label) }
}

$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if ($edge) { Write-Ok "Microsoft Edge" } else { [void](Install-Winget 'Microsoft.Edge' 'Microsoft Edge') }

# Rust no Windows precisa do compilador C++ da Microsoft (Build Tools). Sem ele o build falha com "link.exe".
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasVc = $false
if (Test-Path $vswhere) {
    $vc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $hasVc = [bool]$vc
}
if ($hasVc) {
    Write-Ok "Compilador C++ (Build Tools)"
} else {
    Write-Todo "o compilador C++ e grande (~5 GB) e demora; pode pedir permissao de administrador"
    [void](Install-Winget 'Microsoft.VisualStudio.2022.BuildTools' 'Compilador C++ (Build Tools)' @(
        '--override', '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'))
}

if (Test-Command 'rustup') {
    # so escolhe o toolchain se nao houver nenhum (nao atualiza o Rust de quem ja tem)
    $toolchain = (rustup show active-toolchain 2>$null | Out-String).Trim()
    if ($toolchain) { Write-Ok "Rust: $($toolchain.Split(' ')[0])" } else {
        [void](Invoke-Native "Rust estavel" { rustup default stable })
    }
}

if (Test-Command 'node') {
    if (Test-Command 'pnpm') { Write-Ok "pnpm" } else { [void](Invoke-Native "instalando pnpm" { npm install -g pnpm }) }
    if (Test-Command 'claude') { Write-Ok "Claude Code (claude)" } else {
        [void](Invoke-Native "instalando Claude Code" { npm install -g '@anthropic-ai/claude-code' })
    }
}
Update-SessionPath

# ------------------------------------------------------------------ 3. yt-dlp
Write-Step 3 "yt-dlp (downloads do YouTube) com o solucionador JS"
if (Test-Command 'py') {
    # ja instalado com o solucionador: nao atualiza aqui (pode estar em uso; a tela Primeiros passos tem o botao)
    $ytOk = $false
    if (Test-Command 'yt-dlp') {
        py -3.12 -c "import importlib.metadata as m; m.version('yt-dlp-ejs')" 2>$null
        $ytOk = ($LASTEXITCODE -eq 0)
    }
    if ($ytOk) { Write-Ok "yt-dlp com o solucionador (atualizar: botao Consertar em Primeiros passos)" } else {
        [void](Invoke-Native "instalando yt-dlp" { py -3.12 -m pip install -q -U 'yt-dlp[default]' })
        Update-SessionPath
    }
} else {
    Write-Bad "Python 3.12 nao ficou disponivel; feche esta janela, abra de novo e rode o instalador outra vez"
}

# ------------------------------------------------------------------ 4. Python do projeto
Write-Step 4 "Python do projeto (whisper, streamlink, deteccao de momentos)"
$venv = Join-Path $Root 'tools\rea-video\.venv'
$venvPy = Join-Path $venv 'Scripts\python.exe'
if (Test-Path $venvPy) { Write-Ok "venv ja existe" } elseif (Test-Command 'py') {
    [void](Invoke-Native "criando o venv em tools\rea-video\.venv" { py -3.12 -m venv $venv })
}
if ((Test-Path $venvPy) -or $DryRun) {
    [void](Invoke-Native "instalando os pacotes (scripts\requirements.txt; alguns minutos)" {
        & $venvPy -m pip install -q -U pip
        if ($LASTEXITCODE -eq 0) { & $venvPy -m pip install -q -r (Join-Path $Root 'scripts\requirements.txt') }
    })
    $scrapling = Join-Path $venv 'Scripts\scrapling.exe'
    [void](Invoke-Native "baixando o navegador do Scrapling" { & $scrapling install })
}

# ------------------------------------------------------------------ 5. pacotes Node
Write-Step 5 "Pacotes Node (navegador, videos, app)"
if (Test-Command 'node') {
    [void](Invoke-Native "browser\ (npm ci)" { Push-Location browser; try { npm ci --no-audit --no-fund } finally { Pop-Location } })
    [void](Invoke-Native "video\ (npm ci)" { Push-Location video; try { npm ci --no-audit --no-fund } finally { Pop-Location } })
    [void](Invoke-Native "app\ (pnpm install)" { Push-Location app; try { pnpm install --frozen-lockfile } finally { Pop-Location } })
}

# ------------------------------------------------------------------ 6. configuracao
Write-Step 6 "Configuracao"
$cfg = Join-Path $Root 'tkd.toml'
if (Test-Path $cfg) { Write-Ok "tkd.toml ja existe (nao mexi)" } elseif ($DryRun) { Write-Todo "(simulacao) criaria tkd.toml" } else {
    Copy-Item (Join-Path $Root 'tkd.example.toml') $cfg
    Write-Ok "tkd.toml criado a partir do exemplo"
}

# ------------------------------------------------------------------ 7. compilar
$app = Join-Path $Root 'target\release\tkd-app.exe'
if ($SkipBuild) {
    Write-Step 7 "Compilar: pulado (-SkipBuild)"
} else {
    Write-Step 7 "Compilando o app e a CLI (a primeira vez leva 10 a 30 min)"
    [void](Invoke-Native "CLI tkd (cargo build --release)" { cargo build --release -p tkd })
    [void](Invoke-Native "app desktop (pnpm tauri build)" { Push-Location app; try { pnpm tauri build --no-bundle } finally { Pop-Location } })
}

# ------------------------------------------------------------------ 8. atalho
if (-not $NoShortcut -and ((Test-Path $app) -or $DryRun)) {
    Write-Step 8 "Atalho na area de trabalho"
    $lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'tkd.lnk'
    if ($DryRun) { Write-Todo "(simulacao) criaria $lnk" } else {
        $sh = New-Object -ComObject WScript.Shell
        $s = $sh.CreateShortcut($lnk)
        $s.TargetPath = $app
        $s.WorkingDirectory = $Root
        $s.Description = 'tkd - canais de video curto'
        $s.Save()
        Write-Ok "atalho 'tkd' criado"
    }
}

# ------------------------------------------------------------------ 9. conferencia
Write-Step 9 "Conferencia final (tkd doctor)"
$cli = Join-Path $Root 'target\release\tkd.exe'
if (Test-Path $cli) { & $cli doctor } else { Write-Todo "CLI nao compilada; a conferencia tambem esta na tela 'Primeiros passos' do app" }

Write-Host ""
if ($script:Failed.Count -gt 0) {
    Write-Host "Terminou com $($script:Failed.Count) problema(s):" -ForegroundColor Red
    $script:Failed | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    Write-Host "Feche esta janela, abra de novo e rode o instalador outra vez (o PATH novo so vale em janelas novas)."
    exit 1
}
Write-Host "Pronto." -ForegroundColor Green
Write-Host "Falta so uma coisa: entrar na sua conta do Claude. Abra um terminal novo e digite:  claude"
Write-Host "Depois abra o atalho 'tkd' na area de trabalho e siga a tela 'Primeiros passos'."
