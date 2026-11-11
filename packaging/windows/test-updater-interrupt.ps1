<#
.SYNOPSIS
  Kills the real bdf-updater part-way through applying an update, over and over, and checks recovery.
.DESCRIPTION
  The in-app updater promises "interrupted means rolled back, exactly" and "the previous version is kept whole". This ends the real
  helper process (the same executable BDF starts) at a time swept over the whole replacement, each round on a fresh copy of a managed
  package that holds a lot of files, and then:
    * runs `bdf-updater recover` (what BDF does at its next start) and requires the copy to be exactly the old version, or exactly the
      new one if the kill came after the last rename; never a mixture;
    * requires the person's data (BDFData and a file of their own beside the program) to be byte-for-byte unchanged at every point;
    * applies the update again and requires it to succeed, BDFPrevious to hold the complete old version, and nothing left half-done.
  A process kill is not a power cut (see qualify-windows.ps1 for what a disposable machine adds). Needs a managed copy that trusts a
  development key: build with BDF_UPDATE_PUBKEY and BDF_UPDATE_CHANNEL=dev (the key's file is -KeyFile). Touches only a temp folder.
    powershell -File packaging\windows\test-updater-interrupt.ps1 -BdfDir dist\demo\BDF -KeyFile <update-dev.key> [-Rounds 12] [-Files 300]
#>
param(
    [Parameter(Mandatory)][string]$BdfDir,
    [Parameter(Mandatory)][string]$KeyFile,
    [int]$Rounds = 12,
    [int]$Files = 300,
    [string]$FeedVersion = '0.8.1'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$BdfDir = (Resolve-Path $BdfDir).Path
$KeyFile = (Resolve-Path $KeyFile).Path
$Work = Join-Path ([IO.Path]::GetTempPath()) ("bdf-updater-interrupt-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$failures = 0
function Check([bool]$ok, [string]$what) { if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ } }
function Xtask { & cargo run -q -p xtask -- @args; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } }
function Tree-Hash([string]$Dir, [string[]]$Skip) {
    $r = @{}
    foreach ($f in Get-ChildItem -LiteralPath $Dir -Recurse -File -Force) {
        $rel = $f.FullName.Substring($Dir.Length + 1)
        if ($Skip -and ($Skip -contains $rel.Split('\')[0])) { continue }
        if ($Skip -and $rel -ieq 'notes.txt') { continue }   # the person's own file, checked separately
        $r[$rel.ToLowerInvariant()] = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash
    }
    $r
}
function Same-Tree($A, $B) { ($A.Keys.Count -eq $B.Keys.Count) -and -not @($A.Keys | Where-Object { $B[$_] -ne $A[$_] }) }
$Skip = @('BDFData', 'BDFUpdate', 'BDFPrevious')

try {
    # A bigger package than the real one, so the replacement takes long enough to be interrupted at many different steps: the
    # managed copy's own programs plus a folder of many files, as a newer version, signed with the development key.
    $bin = Join-Path $Work 'bin'; New-Item -ItemType Directory -Path $bin | Out-Null
    foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe') { Copy-Item (Join-Path $BdfDir $n) $bin }
    $models = Join-Path $Work 'models-src'; New-Item -ItemType Directory -Path (Join-Path $models 'models') | Out-Null
    foreach ($i in 1..$Files) { Set-Content -Path (Join-Path $models ('models\m{0:000}.bin' -f $i)) -Value ("model $i " + ('x' * 4000)) -Encoding ascii }
    $rel = Join-Path $Work 'release'
    Push-Location $Root
    Xtask portable-package --bin-dir $bin --models-dir $models --version $FeedVersion --out $rel | Out-Null
    $zip = Join-Path $rel "bdf-$FeedVersion-windows-x64-portable.zip"
    Xtask update-sign --zip $zip --version $FeedVersion --key $KeyFile --out $rel --channel dev --url-base "http://127.0.0.1:1/v$FeedVersion" | Out-Null
    Pop-Location
    $release = Join-Path $rel 'bdf-update.json'
    $newFiles = @{}
    $a = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($e in $a.Entries) {
            if ($e.FullName.EndsWith('/')) { continue }
            $s = $e.Open(); $sha = [Security.Cryptography.SHA256]::Create()
            try { $newFiles[($e.FullName -split '/', 2)[1].Replace('/', '\').ToLowerInvariant()] = ([BitConverter]::ToString($sha.ComputeHash($s)) -replace '-', '') } finally { $s.Dispose(); $sha.Dispose() }
        }
    } finally { $a.Dispose() }

    # The helper is started from a copy of itself, as BDF does, so the one in the folder can be replaced.
    $helper = Join-Path $Work 'helper\bdf-updater.exe'
    New-Item -ItemType Directory -Path (Split-Path $helper) | Out-Null
    Copy-Item (Join-Path $BdfDir 'bdf-updater.exe') $helper

    function Start-Helper([string]$Verb, [string]$App) {
        $argv = @($Verb, '--app-dir', "`"$App`"")
        if ($Verb -eq 'apply') { $argv += @('--release', "`"$release`"", '--zip', "`"$zip`"", '--wait-secs', '5') }
        $p = Start-Process -FilePath $helper -ArgumentList $argv -PassThru -WindowStyle Hidden
        $null = $p.Handle
        $p
    }
    function Run-Helper([string]$Verb, [string]$App) { $p = Start-Helper $Verb $App; $p.WaitForExit(); $p.ExitCode }
    function Moved-Started([string]$App) {
        [bool](Get-ChildItem -LiteralPath (Join-Path $App 'BDFUpdate\previous-new') -Recurse -File -Force -ErrorAction SilentlyContinue | Select-Object -First 1)
    }
    function New-Copy([string]$Dest) {
        Copy-Item -Recurse -LiteralPath $BdfDir -Destination $Dest
        $data = Join-Path $Dest 'BDFData'
        New-Item -ItemType Directory -Path "$data\recovery" -Force | Out-Null
        Set-Content "$data\settings.json" "settings"
        Set-Content "$data\recovery\doc.autosave" "unsaved work"
        Set-Content (Join-Path $Dest 'notes.txt') "my notes"
    }

    # How long from the first rename to the end?
    $probeApp = Join-Path $Work 'probe'; New-Copy $probeApp
    $pp = Start-Helper 'apply' $probeApp
    while (-not (Moved-Started $probeApp) -and -not $pp.HasExited) { Start-Sleep -Milliseconds 1 }
    $sw = [Diagnostics.Stopwatch]::StartNew(); $pp.WaitForExit(); $window = [int]$sw.Elapsed.TotalMilliseconds
    Check ($pp.ExitCode -eq 0) "an uninterrupted update of $($newFiles.Count) files succeeds (exit $($pp.ExitCode))"
    Write-Output ("from the first rename to the end, an update takes about {0} ms; kills are spread over that" -f $window)

    $outcomes = @{ rolledback = 0; finished = 0; untouched = 0 }
    foreach ($round in 1..$Rounds) {
        $app = Join-Path $Work "app$round"; New-Copy $app
        $old = Tree-Hash $app $Skip
        $dataBefore = Tree-Hash (Join-Path $app 'BDFData') @()
        $notesBefore = (Get-FileHash (Join-Path $app 'notes.txt')).Hash
        $delay = [int]($window * ($round - 0.5) / $Rounds)
        $p = Start-Helper 'apply' $app
        while (-not (Moved-Started $app) -and -not $p.HasExited) { Start-Sleep -Milliseconds 1 }
        Start-Sleep -Milliseconds $delay
        if (-not $p.HasExited) { cmd.exe /c "taskkill.exe /PID $($p.Id) /T /F >nul 2>&1" }
        $p.WaitForExit()
        $dataKilled = Tree-Hash (Join-Path $app 'BDFData') @()
        Check ((Same-Tree $dataBefore $dataKilled) -and ((Get-FileHash (Join-Path $app 'notes.txt')).Hash -eq $notesBefore)) "round $round (killed $delay ms after the first rename): the person's data is unchanged at the moment of the kill"

        # what BDF does at its next start
        $rc = Run-Helper 'recover' $app
        $after = Tree-Hash $app $Skip
        $isOld = Same-Tree $old $after
        $isNew = ($newFiles.Keys.Count -eq $after.Keys.Count) -and -not @($newFiles.Keys | Where-Object { $after[$_] -ne $newFiles[$_] })
        $state = if ($isOld) { 'rolledback' } elseif ($isNew) { 'finished' } else { 'MIXED' }
        if ($outcomes.ContainsKey($state)) { $outcomes[$state]++ }
        Check ($rc -eq 0 -and ($isOld -or $isNew)) "round ${round}: after recovery the copy is exactly the old version or exactly the new one (it is: $state)"
        Check (Same-Tree $dataBefore (Tree-Hash (Join-Path $app 'BDFData') @())) "round ${round}: BDFData is unchanged after recovery"

        # apply again, if it had been rolled back
        if ($isOld) {
            $rc2 = Run-Helper 'apply' $app
            $final = Tree-Hash $app $Skip
            $isNew2 = ($newFiles.Keys.Count -eq $final.Keys.Count) -and -not @($newFiles.Keys | Where-Object { $final[$_] -ne $newFiles[$_] })
            Check ($rc2 -eq 0 -and $isNew2) "round ${round}: applying again succeeds and every file is the new version"
            $prevDir = Join-Path $app 'BDFPrevious'
            $prevAll = @{}
            foreach ($f in Get-ChildItem -LiteralPath $prevDir -Recurse -File -Force -ErrorAction SilentlyContinue) { $prevAll[$f.FullName.Substring($prevDir.Length + 1).ToLowerInvariant()] = (Get-FileHash -LiteralPath $f.FullName).Hash }
            $missing = @($old.Keys | Where-Object { $prevAll[$_] -ne $old[$_] })
            Check ($missing.Count -eq 0) "round ${round}: BDFPrevious holds the complete old version ($($missing.Count) of $($old.Count) files missing or different)"
        }
        Check ((Same-Tree $dataBefore (Tree-Hash (Join-Path $app 'BDFData') @())) -and ((Get-FileHash (Join-Path $app 'notes.txt')).Hash -eq $notesBefore)) "round ${round}: the person's data and own file are unchanged at the end"
        Check (-not (Test-Path (Join-Path $app 'BDFUpdate\journal.json')) -and -not (Test-Path (Join-Path $app 'BDFUpdate\stage'))) "round ${round}: no journal or staging folder is left behind"
    }
    Write-Output ("kills landed: rolled back {0}, finished {1}, mixed {2}" -f $outcomes.rolledback, $outcomes.finished, $outcomes.MIXED)
    Check ($outcomes.rolledback -ge 3) 'the sweep really did interrupt the updater part-way through the replacement (at least three times)'
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all updater interruption checks passed'
# Explicit: a native command above may have left $LASTEXITCODE non-zero, and a CI shell would report that instead.
exit 0
