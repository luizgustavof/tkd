# Disco minusculo para os experimentos de "disco cheio" (docs/CHAOS.md, D1). Precisa de PowerShell como ADMINISTRADOR.
#
#   powershell -ExecutionPolicy Bypass -File scripts\chaos_disk.ps1           # cria e monta Q: (12 MB)
#   powershell -ExecutionPolicy Bypass -File scripts\chaos_disk.ps1 -Remove   # desmonta e apaga
#
# O arquivo do disco fica em %LOCALAPPDATA%\tkd-chaos (fora do OneDrive). Depois de montar:
#   $env:TKD_CHAOS_DISK = "Q:\"; tkd chaos run --scenario disk-full-download --channel <canal> --id <video>
param([switch]$Remove, [int]$SizeMB = 12, [string]$Letter = "Q")

$ErrorActionPreference = "Stop"
$dir = Join-Path $env:LOCALAPPDATA "tkd-chaos"
$vhd = Join-Path $dir "small.vhdx"
$script = Join-Path $dir "diskpart.txt"
New-Item -ItemType Directory -Force $dir | Out-Null

if ($Remove) {
    if (-not (Test-Path $vhd)) { Write-Host "nada para remover"; exit 0 }
    "select vdisk file=`"$vhd`"`r`ndetach vdisk" | Set-Content -Encoding ascii $script
    diskpart /s $script | Out-Null
    Remove-Item $vhd -Force
    Write-Host "disco de caos removido"
    exit 0
}

if (Test-Path "${Letter}:\") { throw "a letra ${Letter}: ja esta em uso; use -Letter outra" }
if (Test-Path $vhd) { throw "ja existe $vhd; rode com -Remove antes" }
@"
create vdisk file="$vhd" maximum=$SizeMB type=expandable
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=ntfs quick label=CAOS
assign letter=$Letter
"@ | Set-Content -Encoding ascii $script
diskpart /s $script | Out-Null
$free = (Get-PSDrive $Letter).Free / 1MB
Write-Host ("disco de caos em ${Letter}:\ ({0:N1} MB livres). Use: `$env:TKD_CHAOS_DISK = `"${Letter}:\`"" -f $free)
