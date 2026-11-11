<#
.SYNOPSIS
  Tests the exact portable ZIP a build produced (BUILD-018, R18-3): contents, programs, trust and deployment.

.DESCRIPTION
  Run by package.ps1 on every portable package it builds, and by hand on a downloaded one:

    powershell -File packaging\windows\test-portable-package.ps1 -Zip bdf-0.8.0-windows-x64-portable.zip -Version 0.8.0 -ExpectTrust missing

  It checks the file itself, not a hand-built stand-in:
    1. the package is the whole product and every file matches its own list (`cargo xtask portable-verify`);
    2. the four programs have the right machine type and subsystem;
    3. when this machine can run them: extracted to a fresh folder, bdf-cli reports the version, the copy says it is a
       managed portable copy, the update trust is what -ExpectTrust says (`configured` when the build was given
       BDF_UPDATE_PUBKEY, `missing` when it was not: then every update is refused and the copy says so), nothing is on
       offer, the Windows integration status can be read, and bdf-updater runs (usage error without arguments, a clean
       `recover` on an untouched copy);
    4. the Action1 deployment script installs this same ZIP, under a throwaway test signature, into a fake profile, a
       rerun does nothing, and the installed copy runs.
  Nothing outside a temp folder is touched; nothing is signed with a real key.

.PARAMETER Zip          The portable ZIP to test.
.PARAMETER Version      The version it must carry.
.PARAMETER Arch         x64 (default), x86 or arm64. A package this machine cannot run is inspected, not run.
.PARAMETER ExpectTrust  configured | missing | any.
#>
param(
    [Parameter(Mandatory)][string]$Zip,
    [Parameter(Mandatory)][string]$Version,
    [ValidateSet('x64', 'x86', 'arm64')][string]$Arch = 'x64',
    [ValidateSet('configured', 'missing', 'any')][string]$ExpectTrust = 'any'
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Zip = (Resolve-Path -LiteralPath $Zip).Path
$Work = Join-Path ([IO.Path]::GetTempPath()) ('bdf-package-test-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ }
}
function Xtask {
    # Native stderr is not an error here: the exit code is.
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $out = & cargo run --locked --quiet -p xtask -- @args 2>&1 | Out-String } finally { $ErrorActionPreference = $saved }
    if ($LASTEXITCODE) { throw "xtask $($args[0]) failed: $out" }
    $out
}
function Get-PeHeader([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}

try {
    Push-Location $Root
    # 1. the package is the whole product and matches its own list
    $verified = Xtask portable-verify --zip $Zip --version $Version --arch $Arch
    Check ($verified -match 'files, folder') "the package is complete and every file matches its list: $($verified.Trim())"

    # 2. the programs inside it
    $extract = Join-Path $Work 'extract'
    Expand-Archive -LiteralPath $Zip -DestinationPath $extract
    $top = @(Get-ChildItem -LiteralPath $extract -Directory)
    Check ($top.Count -eq 1 -and @(Get-ChildItem -LiteralPath $extract -File).Count -eq 0) 'the package has exactly one top-level folder'
    $app = $top[0].FullName
    $machine = switch ($Arch) { 'arm64' { 0xAA64 } 'x86' { 0x14C } default { 0x8664 } }
    foreach ($check in @(@('bdf.exe', 2), @('bdf-cli.exe', 3), @('bdf-worker.exe', 3), @('bdf-updater.exe', 2))) {
        $h = Get-PeHeader (Join-Path $app $check[0])
        Check ($h.Machine -eq $machine -and $h.Subsystem -eq $check[1]) "$($check[0]) is $Arch with subsystem $($check[1])"
    }
    Check (Test-Path (Join-Path $app 'portable.txt')) 'portable mode is switched on (portable.txt)'
    Check (-not (Test-Path (Join-Path $app 'BDFData'))) 'the package ships no data folder'

    # 3. run what this machine can run
    $hostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
    $canRun = (($Arch -eq 'x64' -or $Arch -eq 'x86') -and $hostArch -in 'x64', 'arm64', 'x86') -or ($Arch -eq 'arm64' -and $hostArch -eq 'arm64')
    if ($canRun) {
        $cli = Join-Path $app 'bdf-cli.exe'
        $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try {
            $ver = (& $cli --version 2>&1 | Out-String)
            Check ($ver -match [regex]::Escape($Version)) "bdf-cli --version reports $Version"
            $statusText = & $cli update status 2>&1 | Out-String
            $statusCode = $LASTEXITCODE
        } finally { $ErrorActionPreference = $saved }
        $status = $null
        try { $status = $statusText | ConvertFrom-Json } catch { }
        Check ($statusCode -eq 0 -and $null -ne $status) 'bdf-cli update status answers'
        if ($status) {
            Check ($status.current -eq $Version) "the copy reports version $Version"
            Check ($status.managed -eq $true) 'the extracted package is a managed portable copy (it can be updated in place)'
            Check ($null -eq $status.update_available -and -not $status.ready_to_install) 'nothing is on offer from a fresh package'
            $devExpected = ($env:BDF_UPDATE_CHANNEL -eq 'dev')
            Check ($status.dev_build -eq $devExpected) "dev_build is $devExpected (a hosted package is not a development build)"
            if ($ExpectTrust -ne 'any') {
                Check ($status.trust -eq $ExpectTrust) "the update trust is '$ExpectTrust'"
                if ($ExpectTrust -eq 'missing') {
                    Check ($status.check_state -eq 'not_configured') 'a package built without a trust root says it cannot verify updates (not "up to date")'
                    Check ($null -eq $status.key_id) 'no key id without a trust root'
                } else {
                    Check ($status.check_state -in 'never_checked', 'no_feed') 'a package built with a trust root has not checked yet'
                    Check ([bool]$status.key_id) 'a trust root has a key id'
                    if ($env:BDF_UPDATE_PUBKEY) {
                        $expectedId = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([byte[]]@(for ($i = 0; $i -lt $env:BDF_UPDATE_PUBKEY.Length; $i += 2) { [Convert]::ToByte($env:BDF_UPDATE_PUBKEY.Substring($i, 2), 16) })))) -replace '-', ''
                        Check ($expectedId.ToLowerInvariant().StartsWith(([string]$status.key_id).ToLowerInvariant())) 'the key id is the id of the public key the build was given'
                    }
                }
            }
        }
        $savedEap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
        try { $winText = & $cli windows status 2>&1 | Out-String; $winCode = $LASTEXITCODE } finally { $ErrorActionPreference = $savedEap }
        Check ($winCode -eq 0 -and $winText -match 'registered') 'the Windows integration status can be read'
        Check (-not (Test-Path (Join-Path $app 'BDFData\update\update-state.json'))) 'looking at the status wrote no update state'

        $updater = Join-Path $app 'bdf-updater.exe'
        $p = Start-Process -FilePath $updater -PassThru -Wait -WindowStyle Hidden
        Check ($p.ExitCode -eq 2) 'bdf-updater without arguments exits with the usage code and does nothing'
        $p = Start-Process -FilePath $updater -ArgumentList 'recover', '--app-dir', "`"$app`"" -PassThru -Wait -WindowStyle Hidden
        Check ($p.ExitCode -eq 0) 'bdf-updater recover on an untouched copy has nothing to undo and succeeds'
    } else {
        Write-Output "note: not running the $Arch programs on this $hostArch machine; contents and headers were still checked"
    }

    # 4. deploy this same ZIP with the Action1 script, under a throwaway test signature
    if ($Arch -eq 'x64') {
        $keyFile = Join-Path $Work 'test-key.txt'
        $keygen = Xtask update-keygen --out $keyFile
        $pub = (($keygen -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '').Trim()
        $rel = Join-Path $Work 'release'
        Xtask update-sign --zip $Zip --version $Version --key $keyFile --out $rel --channel dev --url-base 'http://127.0.0.1:1/v0' | Out-Null
        $users = Join-Path $Work 'Users'
        New-Item -ItemType Directory -Path (Join-Path $users 'ci') | Out-Null
        $deploy = Join-Path $PSScriptRoot 'deploy-bdf.ps1'
        function Deploy {
            $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
            try {
                $text = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $deploy -PackageZip $Zip -ReleaseFile (Join-Path $rel 'bdf-update.json') -PublicKey $pub -Channel dev `
                    -UsersRoot $users -Users ci -LogDirectory (Join-Path $Work 'log') 2>&1 | Out-String
            } finally { $ErrorActionPreference = $saved }
            [pscustomobject]@{ Code = $LASTEXITCODE; Out = $text }
        }
        $d = Deploy
        $installed = Join-Path $users 'ci\AppData\Local\Programs\BDF'
        Check ($d.Code -eq 0 -and $d.Out -match "installed $([regex]::Escape($Version))") 'the deployment script installs this exact package'
        foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe', 'bdf-package.json', 'portable.txt') {
            Check ((Get-FileHash (Join-Path $installed $n)).Hash -eq (Get-FileHash (Join-Path $app $n)).Hash) "the installed $n is the one in the package"
        }
        $d2 = Deploy
        Check ($d2.Code -eq 0 -and $d2.Out -match 'already has') 'a rerun of the deployment changes nothing'
        if ($canRun) {
            $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
            try { $v2 = & (Join-Path $installed 'bdf-cli.exe') --version 2>&1 | Out-String } finally { $ErrorActionPreference = $saved }
            Check ($v2 -match [regex]::Escape($Version)) 'the deployed copy runs'
        }
    }
} finally {
    Pop-Location
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures package check(s) failed"; exit 1 }
Write-Output 'all package checks passed'
