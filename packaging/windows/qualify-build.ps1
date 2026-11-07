<#
.SYNOPSIS
  Builds the development-signed material the Windows qualification runs on (BUILD-018).
.DESCRIPTION
  For two versions (the workspace version and the next patch) this builds the four programs with a throwaway update signing key
  compiled in (BDF_UPDATE_PUBKEY, channel dev: they trust only that key and only read a feed from a loopback address), packages
  them like the release does, and signs the packages with the key. Nothing here is a release and nothing is published.
  Output folder:
    bins\<version>\            bdf.exe, bdf-cli.exe, bdf-worker.exe, bdf-updater.exe
    packages\<version>\        the portable ZIP and its bdf-update.json (signed with the throwaway key)
    feed\                      release.json and v<next>\<zip>: a local test feed offering the newer version on port -Port
    pubkey.txt                 the throwaway public key (the private key is written to -KeyFile, OUTSIDE this folder, and never uploaded)
  Run from the repository root:
    powershell -File packaging\windows\qualify-build.ps1 -Out C:\qual -KeyFile C:\qual-private\dev.key
#>
param(
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$KeyFile,
    [int]$Port = 8089,
    [string]$Version0,
    [string]$Version1
)
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $Repo
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
if (-not $Version0) { $Version0 = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value }
if (-not $Version1) { $p = $Version0.Split('.'); $Version1 = "$($p[0]).$($p[1]).$([int]$p[2] + 1)" }
function Xtask { & cargo run --locked -q -p xtask -- @args; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } }
New-Item -ItemType Directory -Path $Out -Force | Out-Null
New-Item -ItemType Directory -Path (Split-Path $KeyFile -Parent) -Force | Out-Null
$keygen = Xtask update-keygen --out $KeyFile | Out-String
$pub = (($keygen -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '').Trim()
if ($pub -notmatch '^[0-9a-fA-F]{130}$') { throw 'could not read the public key from update-keygen' }
Set-Content -Path (Join-Path $Out 'pubkey.txt') -Value $pub -Encoding ascii
$env:BDF_UPDATE_PUBKEY = $pub
$env:BDF_UPDATE_CHANNEL = 'dev'
$names = 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe'
try {
    foreach ($v in $Version0, $Version1) {
        Xtask version set $v
        # Cargo reuses workspace artifacts across a version change and leaves the old executables: force a real rebuild of ours.
        Get-ChildItem (Join-Path $Repo 'target\release\.fingerprint') -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like 'pdfcraft*' -or $_.Name -like 'bdf*' } | Remove-Item -Recurse -Force
        foreach ($n in $names) { Remove-Item (Join-Path $Repo "target\release\$n") -ErrorAction SilentlyContinue }
        & cargo build --release --locked -p pdfcraft -p pdfcraft-cli -p pdfcraft-worker -p pdfcraft-updater
        if ($LASTEXITCODE) { throw "build $v failed" }
        $bin = Join-Path $Out "bins\$v"
        New-Item -ItemType Directory -Path $bin -Force | Out-Null
        foreach ($n in $names) { Copy-Item (Join-Path $Repo "target\release\$n") $bin }
        $reported = (& (Join-Path $bin 'bdf-cli.exe') --version | Select-Object -First 1)
        if ($reported -notmatch [regex]::Escape($v)) { throw "the build for $v reports '$reported'" }
        Write-Output "built $v ($reported)"
        $pkg = Join-Path $Out "packages\$v"
        Xtask portable-package --bin-dir $bin --version $v --out $pkg
        $zip = Join-Path $pkg "bdf-$v-windows-x64-portable.zip"
        $base = if ($v -eq $Version1) { "http://127.0.0.1:$Port/v$v" } else { "http://127.0.0.1:1/v$v" }
        Xtask update-sign --zip $zip --version $v --key $KeyFile --out $pkg --channel dev --url-base $base
    }
} finally {
    Xtask version set $Version0
}
# the local test feed that offers the newer version
$feed = Join-Path $Out 'feed'
New-Item -ItemType Directory -Path (Join-Path $feed "v$Version1") -Force | Out-Null
Copy-Item (Join-Path $Out "packages\$Version1\bdf-update.json") (Join-Path $feed 'release.json')
Copy-Item (Join-Path $Out "packages\$Version1\bdf-$Version1-windows-x64-portable.zip") (Join-Path $feed "v$Version1")
Set-Content -Path (Join-Path $Out 'versions.txt') -Value "$Version0`n$Version1" -Encoding ascii
Write-Output "qualification material for $Version0 and $Version1 is in $Out"
