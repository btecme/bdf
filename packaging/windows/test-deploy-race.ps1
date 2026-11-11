<#
.SYNOPSIS
  Measures whether deploy-bdf.ps1 can be steered through a link that a person swaps in while it runs.
.DESCRIPTION
  Deploys an update over an installed copy that has a folder of many files, while a second runspace tries as fast as it can to
  replace that folder with a junction to an "outside" folder holding files of the same names. If any outside file is moved,
  replaced or deleted, or gets a new neighbour, the run was steered through the link. Prints one line per trial and a total.
  Touches nothing outside a temp folder; needs no elevation (a junction is an ordinary user's tool).

  The result is a measurement, not a proof: zero hits in N trials does not show there is no window. It exists so that a
  claim about a deployment mode can be checked against an attacker instead of against the reader's confidence.
  Run from the repository root: powershell -File packaging\windows\test-deploy-race.ps1 [-Trials 12] [-Files 60]
#>
param([int]$Trials = 12, [int]$Files = 60, [string]$DeployScript, [switch]$ExpectNoHits)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $DeployScript) { $DeployScript = Join-Path $PSScriptRoot 'deploy-bdf.ps1' }
$Work = Join-Path ([IO.Path]::GetTempPath()) ("bdf-race-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
function Xtask { & cargo run -q -p xtask -- @args; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } }

$attacker = {
    param($app, $outside, $stop)
    $dir = Join-Path $app 'models'
    $real = Join-Path $app 'models.real'
    # wait until the deployment has got past its up-front link scan (its working folder appears)
    $gate = Join-Path $app 'BDFUpdate\deploy-stage'
    $until = [DateTime]::UtcNow.AddSeconds(60)
    while (-not (Test-Path -LiteralPath $gate) -and [DateTime]::UtcNow -lt $until -and -not $stop.Value) { }
    while (-not $stop.Value) {
        try {
            [IO.Directory]::Move($dir, $real)
            try { New-Item -ItemType Junction -Path $dir -Target $outside | Out-Null } catch { }
            [IO.Directory]::Delete($dir)
        } catch { }
        try { if (-not (Test-Path -LiteralPath $dir) -and (Test-Path -LiteralPath $real)) { [IO.Directory]::Move($real, $dir) } } catch { }
    }
    try { if ((Test-Path -LiteralPath $dir) -and ((Get-Item -LiteralPath $dir -Force).Attributes -band 'ReparsePoint')) { [IO.Directory]::Delete($dir) } } catch { }
    try { if (-not (Test-Path -LiteralPath $dir) -and (Test-Path -LiteralPath $real)) { [IO.Directory]::Move($real, $dir) } } catch { }
}

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
        foreach ($i in 1..$Files) { Set-Content -Path (Join-Path $models ('models\m{0:000}.bin' -f $i)) -Value "model $i of $Version" -Encoding ascii }
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
    function Run-Deploy([string[]]$Extra) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $DeployScript @Extra 2>&1 | Out-String
    }

    $hits = 0; $refused = 0; $updated = 0
    foreach ($t in 1..$Trials) {
        $users = Join-Path $Work "Users$t"
        $app = Join-Path $users 'victim\AppData\Local\Programs\BDF'
        New-Item -ItemType Directory -Path (Join-Path $users 'victim') | Out-Null
        $outside = Join-Path $Work "outside$t"
        New-Item -ItemType Directory -Path $outside | Out-Null
        $deployArgs = @('-PublicKey', $pub, '-Channel', 'dev', '-UsersRoot', $users, '-Users', 'victim')
        Run-Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $deployArgs) | Out-Null
        # the files the outside folder holds are the ones the update would move or replace
        foreach ($i in 1..$Files) { Set-Content -Path (Join-Path $outside ('m{0:000}.bin' -f $i)) -Value "CANARY $i" -Encoding ascii }
        $before = @{}
        foreach ($f in Get-ChildItem $outside -File) { $before[$f.Name] = (Get-FileHash $f.FullName).Hash }

        $stop = [ref]$false
        $rs = [runspacefactory]::CreateRunspace(); $rs.Open()
        $ps = [powershell]::Create(); $ps.Runspace = $rs
        [void]$ps.AddScript($attacker).AddArgument($app).AddArgument($outside).AddArgument($stop)
        # Anything that happens in the outside folder is a hit, even when a rollback later puts it back.
        $events = [System.Collections.Concurrent.ConcurrentBag[string]]::new()
        $watch = New-Object System.IO.FileSystemWatcher $outside
        $watch.IncludeSubdirectories = $true
        $watch.NotifyFilter = 'FileName, DirectoryName, LastWrite, Size, CreationTime, Attributes'
        $watch.InternalBufferSize = 65536
        $sink = { $Event.MessageData.Add($Event.SourceEventArgs.ChangeType.ToString() + " " + $Event.SourceEventArgs.Name) }
        $subs = foreach ($n in 'Created', 'Deleted', 'Changed', 'Renamed') { Register-ObjectEvent -InputObject $watch -EventName $n -Action $sink -MessageData $events }
        $watch.EnableRaisingEvents = $true
        $handle = $ps.BeginInvoke()
        $out = Run-Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $deployArgs)
        $stop.Value = $true
        [void]$ps.EndInvoke($handle); $ps.Dispose(); $rs.Dispose()
        Start-Sleep -Milliseconds 300
        $watch.EnableRaisingEvents = $false
        foreach ($s in $subs) { Unregister-Event -SubscriptionId $s.Id -ErrorAction SilentlyContinue }
        $watch.Dispose()

        $after = @{}
        foreach ($f in Get-ChildItem $outside -File -Force) { $after[$f.Name] = (Get-FileHash $f.FullName).Hash }
        $changed = @($before.Keys | Where-Object { $after[$_] -ne $before[$_] }).Count
        $extra = @($after.Keys | Where-Object { -not $before.ContainsKey($_) }).Count
        $seen = $events.Count; $evText = ($events.ToArray() | Select-Object -First 3) -join '; '
        $hit = ($changed + $extra + $seen) -gt 0
        if ($hit) { $hits++ }
        if ($out -match 'refusing|link or junction|a link appeared|FAILED') { $refused++ }
        if ($out -match 'updated 0.8.1 -> 0.8.2') { $updated++ }
        $why = (($out -split "`n" | Where-Object { $_ -match 'victim:' } | Select-Object -First 1) -replace '^\S+ \S+ ', '').Trim()
        if ($why.Length -gt 110) { $why = $why.Substring(0, 110) }
        '{0,3}: outside: {1} files differ at the end, {2} new, {3} change events ({6}) -> {4}  [{5}]' -f $t, $changed, $extra, $seen, $(if ($hit) { 'STEERED THROUGH THE LINK' } else { 'outside untouched' }), $why, $evText
    }
    "trials $Trials, steered through the link $hits, runs that refused or failed $refused, runs that updated $updated"
    if ($ExpectNoHits -and $hits -gt 0) { exit 1 }
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
