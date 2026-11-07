<#
.SYNOPSIS
  Kills deploy-bdf.ps1 part-way through an update, over and over at different moments, and checks that the next run finishes the job.
.DESCRIPTION
  A real process kill, not a simulated failure: a package of many files is deployed over an installed older copy, the
  PowerShell process is ended a chosen time after it first changes the person's copy, and then the deployment is run again.
  After every kill:
    * the person's own data (BDFData, a file of their own) is exactly as it was;
    * the next run succeeds, reports an update (never "already current" for a copy left half-way), and afterwards every file
      hashes to the new release, no marker or staging folder is left, and a further run finds nothing to do;
    * when the kill landed part-way, BDFPrevious holds the complete version the person had, so the in-app rollback still works.
  A process kill is not a power cut: the file system finishes what it had been asked to do. What it does exercise is every
  point in the script's own sequence of steps, which is where the script's recovery has to work. A power cut itself, and a
  full disk, are separate checks (qualify-windows.ps1) that need a disposable machine.
  Touches nothing outside a temp folder. Run from the repository root:
    powershell -File packaging\windows\test-deploy-interrupt.ps1 [-Rounds 14] [-Files 250] [-DeployScript path]
  -DeployScript runs the same checks against another version of the script (for showing that a check fails without a fix).
#>
param([int]$Rounds = 14, [int]$Files = 250, [string]$DeployScript)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Deploy = if ($DeployScript) { $DeployScript } else { Join-Path $PSScriptRoot 'deploy-bdf.ps1' }
$Work = Join-Path ([IO.Path]::GetTempPath()) ("bdf-interrupt-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$failures = 0
function Check([bool]$ok, [string]$what) { if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ } }
function Xtask { & cargo run -q -p xtask -- @args; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } }
function Tree-Hash([string]$Dir, [string[]]$Skip) {
    # name -> hash for every file under Dir except the named top-level folders
    $r = @{}
    foreach ($f in Get-ChildItem -LiteralPath $Dir -Recurse -File -Force) {
        $rel = $f.FullName.Substring($Dir.Length + 1)
        if ($Skip -and ($Skip -contains $rel.Split('\')[0])) { continue }
        $r[$rel.ToLowerInvariant()] = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash
    }
    $r
}
function Zip-Hashes([string]$Zip) {
    $r = @{}
    $a = [IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        foreach ($e in $a.Entries) {
            if ($e.FullName.EndsWith('/')) { continue }
            $s = $e.Open(); $sha = [Security.Cryptography.SHA256]::Create()
            try { $r[($e.FullName -split '/', 2)[1].Replace('/', '\').ToLowerInvariant()] = ([BitConverter]::ToString($sha.ComputeHash($s)) -replace '-', '') } finally { $s.Dispose(); $sha.Dispose() }
        }
    } finally { $a.Dispose() }
    $r
}
function Same-Tree($A, $B) { ($A.Keys.Count -eq $B.Keys.Count) -and -not @($A.Keys | Where-Object { $B[$_] -ne $A[$_] }) }

try {
    Push-Location $Root
    $keyFile = Join-Path $Work 'key.txt'
    $keygen = Xtask update-keygen --out $keyFile | Out-String
    $pub = (($keygen -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '').Trim()
    $models = Join-Path $Work 'models-src'
    New-Item -ItemType Directory -Path (Join-Path $models 'models') | Out-Null
    $bin = Join-Path $Work 'bin'
    New-Item -ItemType Directory -Path $bin | Out-Null
    function Make-Release([string]$Version) {
        foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe') { Set-Content -Path (Join-Path $bin $n) -Value "$n $Version" -Encoding ascii }
        foreach ($i in 1..$Files) { Set-Content -Path (Join-Path $models ('models\m{0:000}.bin' -f $i)) -Value ("model $i of $Version " + ('x' * 2000)) -Encoding ascii }
        $out = Join-Path $Work "release-$Version"
        Xtask portable-package --bin-dir $bin --models-dir $models --version $Version --out $out | Out-Null
        Xtask update-sign --zip (Join-Path $out "bdf-$Version-windows-x64-portable.zip") --version $Version --key $keyFile --out $out --channel dev --url-base "http://127.0.0.1:1/v$Version" | Out-Null
        $out
    }
    $r1 = Make-Release '0.8.1'
    $r2 = Make-Release '0.8.2'
    Pop-Location
    $zip1 = Join-Path $r1 'bdf-0.8.1-windows-x64-portable.zip'; $rel1 = Join-Path $r1 'bdf-update.json'
    $zip2 = Join-Path $r2 'bdf-0.8.2-windows-x64-portable.zip'; $rel2 = Join-Path $r2 'bdf-update.json'
    $zip1Files = Zip-Hashes $zip1
    $zip2Files = Zip-Hashes $zip2

    function Start-Deploy([string]$Zip, [string]$Rel, [string]$Users, [string]$Name) {
        $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$Deploy`"", '-PackageZip', "`"$Zip`"", '-ReleaseFile', "`"$Rel`"",
            '-PublicKey', $pub, '-Channel', 'dev', '-UsersRoot', "`"$Users`"", '-Users', $Name)
        $p = Start-Process -FilePath powershell.exe -ArgumentList $a -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Work ('out-' + [guid]::NewGuid().ToString('N') + '.txt'))
        $null = $p.Handle   # without this, ExitCode is null after the process ends
        $p
    }
    function Run-Deploy([string]$Zip, [string]$Rel, [string]$Users, [string]$Name) {
        $p = Start-Deploy $Zip $Rel $Users $Name
        $p.WaitForExit()
        $p.ExitCode
    }
    function Last-Output { Get-ChildItem $Work -Filter 'out-*.txt' | Sort-Object LastWriteTime | Select-Object -Last 1 | Get-Content -Raw }
    # The first change to the person's copy: the first old file set aside.
    function Aside-Started([string]$App) {
        $prev = Join-Path $App 'BDFUpdate\deploy-previous'
        [bool](Get-ChildItem -LiteralPath $prev -Recurse -File -Force -ErrorAction SilentlyContinue | Select-Object -First 1)
    }

    # How long from the first change to the end? The kill delays are spread over that, so the kills land in every phase.
    $probe = Join-Path $Work 'UsersProbe'; New-Item -ItemType Directory -Path (Join-Path $probe 'p') | Out-Null
    [void](Run-Deploy $zip1 $rel1 $probe 'p')
    $probeApp = Join-Path $probe 'p\AppData\Local\Programs\BDF'
    $pp = Start-Deploy $zip2 $rel2 $probe 'p'
    while (-not (Aside-Started $probeApp) -and -not $pp.HasExited) { Start-Sleep -Milliseconds 2 }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $pp.WaitForExit()
    $window = [int]$sw.Elapsed.TotalMilliseconds
    Write-Output ("from the first change to the end, an update takes about {0:n0} ms; kills are spread over that" -f $window)

    $outcomes = @{ untouched = 0; partial = 0; complete = 0 }
    foreach ($round in 1..$Rounds) {
        $users = Join-Path $Work "Users$round"
        $name = 'victim'
        New-Item -ItemType Directory -Path (Join-Path $users $name) | Out-Null
        $app = Join-Path $users "$name\AppData\Local\Programs\BDF"
        [void](Run-Deploy $zip1 $rel1 $users $name)
        # the person's own things
        $data = Join-Path $app 'BDFData'
        New-Item -ItemType Directory -Path "$data\recovery" -Force | Out-Null
        Set-Content "$data\settings.json" "settings $round"
        Set-Content "$data\recovery\doc.autosave" "unsaved work $round"
        Set-Content (Join-Path $app 'notes.txt') "my notes $round"
        $oldHashes = Tree-Hash $app @('BDFData', 'BDFUpdate', 'BDFPrevious')
        $dataHashes = Tree-Hash $data @()

        $delay = [int]($window * ($round - 0.5) / $Rounds)
        $p = Start-Deploy $zip2 $rel2 $users $name
        while (-not (Aside-Started $app) -and -not $p.HasExited) { Start-Sleep -Milliseconds 2 }
        Start-Sleep -Milliseconds $delay
        if (-not $p.HasExited) { cmd.exe /c "taskkill.exe /PID $($p.Id) /T /F >nul 2>&1" }
        $p.WaitForExit()

        $marker = Join-Path $app 'BDFUpdate\deploy-pending.json'
        $keepDir = Join-Path $app 'BDFPrevious'
        $now = Tree-Hash $app @('BDFData', 'BDFUpdate', 'BDFPrevious')
        # partial: some old file is set aside and the version the person had is not yet in BDFPrevious
        $partial = (Test-Path $marker) -or ((Aside-Started $app) -and -not (Test-Path $keepDir))
        $state = if ($partial) { 'partial' } elseif (Same-Tree $oldHashes $now) { 'untouched' } else { 'complete' }
        $outcomes[$state]++
        $dataNow = Tree-Hash $data @()
        Check ((Same-Tree $dataHashes $dataNow) -and ((Get-Content (Join-Path $app 'notes.txt')) -eq "my notes $round")) "round $round (killed $delay ms after the first change, state: $state): the person's data and own file are exactly as they were"

        # the next run
        $code = Run-Deploy $zip2 $rel2 $users $name
        $log = Last-Output
        $new = Tree-Hash $app @('BDFData', 'BDFUpdate', 'BDFPrevious')
        $missingNew = @($zip2Files.Keys | Where-Object { $new[$_] -ne $zip2Files[$_] })
        Check ($code -eq 0 -and $missingNew.Count -eq 0) "round ${round}: the next run succeeds and every file is the new release ($($missingNew.Count) wrong)"
        if ($state -eq 'partial') { Check ($log -match 'updated 0.8.[12] -> 0.8.2') "round ${round}: the next run says it updated, not that the copy is current" }
        Check (-not (Test-Path $marker) -and -not (Test-Path (Join-Path $app 'BDFUpdate\deploy-stage'))) "round ${round}: no marker or staging folder is left behind"
        Check (Same-Tree $dataHashes (Tree-Hash $data @())) "round ${round}: BDFData is still untouched after the finishing run"
        if ($state -eq 'partial') {
            # the version the person had, complete and byte for byte, so the in-app rollback has something to return to
            $prevAll = @{}
            foreach ($f in Get-ChildItem -LiteralPath $keepDir -Recurse -File -Force -ErrorAction SilentlyContinue) {
                $prevAll[$f.FullName.Substring($keepDir.Length + 1).ToLowerInvariant()] = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash
            }
            $missing = @($zip1Files.Keys | Where-Object { $prevAll[$_] -ne $zip1Files[$_] })
            Check ($missing.Count -eq 0) "round ${round}: BDFPrevious holds the complete version the person had ($($missing.Count) of $($zip1Files.Count) files missing or different: $(($missing | Select-Object -First 3) -join ', '))"
        }
        # a further run changes nothing
        $code3 = Run-Deploy $zip2 $rel2 $users $name
        Check ($code3 -eq 0 -and (Last-Output) -match 'already has') "round ${round}: a further run finds nothing to do"
    }
    Write-Output ("kills landed: before any change {0}, part-way {1}, after the last step {2}" -f $outcomes.untouched, $outcomes.partial, $outcomes.complete)
    Check ($outcomes.partial -ge 3) 'the sweep really did kill the script part-way through an update (at least three times)'
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all interruption checks passed'
# Explicit: a native command above may have left $LASTEXITCODE non-zero, and a CI shell would report that instead.
exit 0
