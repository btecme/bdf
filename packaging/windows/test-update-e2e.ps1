<#
.SYNOPSIS
  End-to-end update test with real binaries: extract, check, download, apply, two successive upgrades, tampered and
  forged releases, offline use, and an update that waits for a running program. Uses a local feed and synthetic data only.
.DESCRIPTION
  Needs three builds of the same source at versions 0.8.0, 0.8.1 and 0.8.2, made with a *development* signing key
  (BDF_UPDATE_PUBKEY / BDF_UPDATE_CHANNEL=dev), in <BinRoot>\<version>\ (bdf.exe, bdf-cli.exe, bdf-worker.exe,
  bdf-updater.exe), and the private key file. Everything runs in a temp folder as the current user.
  powershell -File packaging\windows\test-update-e2e.ps1 -BinRoot target\preview-bins -KeyFile <dev key> -Out <evidence.txt>
#>
param(
    [Parameter(Mandatory)][string]$BinRoot,
    [Parameter(Mandatory)][string]$KeyFile,
    [string]$Out
)
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$BinRoot = (Resolve-Path $BinRoot).Path
$Work = Join-Path ([IO.Path]::GetTempPath()) ("bdf-e2e-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$failures = 0
$lines = New-Object System.Collections.ArrayList
function Installed-Version([string]$dir) {
    $m = (Get-Content (Join-Path $dir 'bdf-package.json') -Raw -ErrorAction SilentlyContinue) | Select-String '"version": "([0-9.]+)"'
    if ($m) { $m.Matches[0].Groups[1].Value } else { '' }
}
function Say([string]$m) { Write-Output $m; [void]$lines.Add($m) }
function Check([bool]$ok, [string]$what) { if ($ok) { Say "ok   $what" } else { Say "FAIL $what"; $script:failures++ } }
function Xtask { Push-Location $Repo; try { & cargo run -q -p xtask -- @args | Out-Null; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } } finally { Pop-Location } }
function BdfCli([string]$Dir, [string[]]$a) {
    # A failing command writes to stderr, which Windows PowerShell 5.1 turns into a terminating error under 'Stop'.
    $previous = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $o = & (Join-Path $Dir 'bdf-cli.exe') @a 2>&1 | Out-String; $code = $LASTEXITCODE } finally { $ErrorActionPreference = $previous }
    [pscustomobject]@{ Code = $code; Out = $o }
}
function Json([string]$text) { $text | ConvertFrom-Json }
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 60) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 300 }
    $false
}
function Stop-RestartedBdf([string]$Dir) {
    # The updater starts BDF again after an update (as it does for the app); wait for it, then stop this test instance.
    Wait-Until { @(Get-Process bdf -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Dir*" }).Count -gt 0 } 20 | Out-Null
    Get-Process bdf -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Dir*" } | Stop-Process -Force
    Start-Sleep -Milliseconds 800
}
$server = $null
try {
    # ---- packages and feed -----------------------------------------------------------------------------------
    $port = Get-Random -Minimum 20000 -Maximum 40000
    $feedRoot = Join-Path $Work 'feed'
    New-Item -ItemType Directory -Path $feedRoot | Out-Null
    $zips = @{}
    foreach ($v in '0.8.0', '0.8.1', '0.8.2') {
        $relDir = Join-Path $Work "rel-$v"
        Xtask portable-package --bin-dir (Join-Path $BinRoot $v) --version $v --out $relDir
        Xtask update-sign --zip (Join-Path $relDir "bdf-$v-windows-x64-portable.zip") --version $v --key $KeyFile --out $relDir --channel dev --url-base "http://127.0.0.1:$port/v$v"
        New-Item -ItemType Directory -Path (Join-Path $feedRoot "v$v") | Out-Null
        Copy-Item (Join-Path $relDir "bdf-$v-windows-x64-portable.zip") (Join-Path $feedRoot "v$v")
        $zips[$v] = Join-Path $relDir "bdf-$v-windows-x64-portable.zip"
        Copy-Item (Join-Path $relDir 'bdf-update.json') (Join-Path $Work "release-$v.json")
    }
    function Publish([string]$v) { Copy-Item (Join-Path $Work "release-$v.json") (Join-Path $feedRoot 'release.json') -Force }
    Publish '0.8.0'
    $py = (Get-Command python -ErrorAction Stop).Source
    $server = Start-Process -FilePath $py -ArgumentList @('-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', $feedRoot) -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    $env:BDF_UPDATE_FEED = "http://127.0.0.1:$port/release.json"

    # ---- extract and run as the current (standard) user ---------------------------------------------------------
    $app = Join-Path $Work 'BDF'
    Expand-Archive -Path $zips['0.8.0'] -DestinationPath $Work
    Move-Item (Join-Path $Work 'bdf-0.8.0-windows-x64') $app
    Check (Test-Path (Join-Path $app 'portable.txt')) 'the extracted folder runs as is (portable.txt present)'
    $v = (BdfCli $app @('--version')).Out
    Check ($v -match '0\.8\.0') "bdf-cli reports 0.8.0 ($($v.Trim()))"
    $st = Json (BdfCli $app @('update', 'status')).Out
    Check ($st.managed -and $st.dev_build -and $st.trust -eq 'configured' -and -not $st.update_available) 'a managed dev copy with a trust key and nothing on offer'
    $files = Get-ChildItem $app -File | ForEach-Object Name
    foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe', 'portable.txt', 'LICENSE-MIT', 'NOTICE', 'README.txt', 'bdf-package.json') { Check ($files -contains $n) "package contains $n" }
    Check (-not (Test-Path (Join-Path $app 'BDFData')) -or @(Get-ChildItem (Join-Path $app 'BDFData') -Recurse -File).Count -eq 0) 'the package ships no user data'

    # user data that must survive every update
    $data = Join-Path $app 'BDFData'
    New-Item -ItemType Directory -Path "$data\recovery" -Force | Out-Null
    Set-Content "$data\settings.json" '{"theme_choice":"dark"}' -Encoding ascii
    Set-Content "$data\recovery\doc1.autosave" 'unsaved work' -Encoding ascii
    Set-Content (Join-Path $app 'my-notes.txt') 'mine' -Encoding ascii
    function Data-Hash { (Get-ChildItem "$data\settings.json", "$data\recovery\doc1.autosave", (Join-Path $app 'my-notes.txt') | Get-FileHash -Algorithm SHA256 | ForEach-Object Hash) -join ',' }
    $before = Data-Hash

    # ---- nothing newer: no offer -------------------------------------------------------------------------------
    $c = BdfCli $app @('update', 'check')
    Check ($c.Code -eq 0 -and -not (Json $c.Out).status.update_available) 'a feed at the same version offers nothing'

    # ---- upgrade 1: 0.8.0 -> 0.8.1 --------------------------------------------------------------------------------
    Publish '0.8.1'
    $c = BdfCli $app @('update', 'check')
    $offer = (Json $c.Out).status.update_available
    Check ($offer.version -eq '0.8.1') 'check finds 0.8.1'
    $c = BdfCli $app @('update', 'download')
    Check ($c.Code -eq 0 -and (Json $c.Out).ready) 'download verifies the package'
    $c = BdfCli $app @('update', 'apply')
    Check ($c.Code -eq 0 -and (Json $c.Out).scheduled) 'apply hands over to the updater'
    $ok = Wait-Until { (Installed-Version $app) -eq '0.8.1' } 90
    # the updater restarts BDF after an update, as it does for the app: stop that test instance
    Stop-RestartedBdf $app
    Check $ok 'the updater reports "updated"'
    Check ((BdfCli $app @('--version')).Out -match '0\.8\.1') 'bdf-cli now reports 0.8.1'
    Check ((Data-Hash) -eq $before) 'BDFData, recovery file and the user file are byte-identical after update 1'
    Check (Test-Path (Join-Path $app 'BDFPrevious\bdf.exe')) 'the previous version is kept (BDFPrevious)'
    Check ((Get-Content (Join-Path $app 'bdf-package.json') -Raw) -match '"version": "0.8.1"') 'the installed list is 0.8.1'

    # ---- upgrade 2: 0.8.1 -> 0.8.2 ----------------------------------------------------------------------------------
    Remove-Item (Join-Path $app 'BDFUpdate\last-result.json') -ErrorAction SilentlyContinue
    Publish '0.8.2'
    BdfCli $app @('update', 'check') | Out-Null
    BdfCli $app @('update', 'download') | Out-Null
    BdfCli $app @('update', 'apply') | Out-Null
    $ok = Wait-Until { (Installed-Version $app) -eq '0.8.2' } 90
    Stop-RestartedBdf $app
    Check ($ok -and (BdfCli $app @('--version')).Out -match '0\.8\.2') 'second successive upgrade: 0.8.2'
    Check ((Data-Hash) -eq $before) 'user data is byte-identical after update 2'
    Check ((Get-Content (Join-Path $app 'BDFPrevious\bdf-package.json') -Raw) -match '"version": "0.8.1"') 'BDFPrevious now holds 0.8.1'

    # ---- rollback --------------------------------------------------------------------------------------------------
    Remove-Item (Join-Path $app 'BDFUpdate\last-result.json') -ErrorAction SilentlyContinue
    BdfCli $app @('update', 'rollback') | Out-Null
    $ok = Wait-Until { (Installed-Version $app) -eq '0.8.1' } 90
    Stop-RestartedBdf $app
    Check ($ok -and (BdfCli $app @('--version')).Out -match '0\.8\.1') 'rollback returns to 0.8.1'
    Check ((Data-Hash) -eq $before) 'user data is byte-identical after rollback'

    # ---- tampered package and forged release ---------------------------------------------------------------------------
    $app2 = Join-Path $Work 'BDF2'
    Expand-Archive -Path $zips['0.8.0'] -DestinationPath $Work
    Move-Item (Join-Path $Work 'bdf-0.8.0-windows-x64') $app2
    Publish '0.8.2'
    $pkg = Join-Path $feedRoot 'v0.8.2\bdf-0.8.2-windows-x64-portable.zip'
    $good = [IO.File]::ReadAllBytes($pkg)
    $bad = $good.Clone(); $bad[[int]($bad.Length / 2)] = $bad[[int]($bad.Length / 2)] -bxor 0x55
    [IO.File]::WriteAllBytes($pkg, $bad)
    BdfCli $app2 @('update', 'check') | Out-Null
    $c = BdfCli $app2 @('update', 'download')
    Check ($c.Code -ne 0 -and $c.Out -match 'failed verification') 'a tampered package fails verification and is discarded'
    Check (-not (Get-ChildItem (Join-Path $app2 'BDFUpdate\downloads') -Filter *.zip -ErrorAction SilentlyContinue)) 'no tampered package is left to install'
    [IO.File]::WriteAllBytes($pkg, $good)
    # A forged signature, on a fresh copy that has never seen a genuine release (an earlier verified offer would rightly stay).
    $app3 = Join-Path $Work 'BDF3'
    Expand-Archive -Path $zips['0.8.0'] -DestinationPath $Work
    Move-Item (Join-Path $Work 'bdf-0.8.0-windows-x64') $app3
    $forged = (Get-Content (Join-Path $Work 'release-0.8.2.json') -Raw) -replace '"signature": "', '"signature": "0'
    [IO.File]::WriteAllText((Join-Path $feedRoot 'release.json'), $forged, (New-Object System.Text.UTF8Encoding $false))
    $c = BdfCli $app3 @('update', 'check')
    Check ($c.Code -ne 0 -and $c.Out -match 'verification') 'a release with a forged signature is rejected'
    Check (-not (Json (BdfCli $app3 @('update', 'status')).Out).update_available) 'and nothing is offered'
    # A release whose payload was changed after signing (the signature no longer matches).
    $env0 = Get-Content (Join-Path $Work 'release-0.8.2.json') -Raw | ConvertFrom-Json
    $payload = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env0.payload)) -replace '"size": (\d+)', '"size": 1$1'
    $env0.payload = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($payload))
    [IO.File]::WriteAllText((Join-Path $feedRoot 'release.json'), ($env0 | ConvertTo-Json), (New-Object System.Text.UTF8Encoding $false))
    $c = BdfCli $app3 @('update', 'check')
    Check ($c.Code -ne 0 -and $c.Out -match 'verification') 'a release whose signed information was altered is rejected'

    # ---- an older release is never installed ------------------------------------------------------------------------------
    Publish '0.8.0'
    $c = BdfCli $app @('update', 'check')
    Check ($c.Code -eq 0 -and (Json $c.Out).status.rejected -match 'older|already seen') 'an older release in the feed is rejected, not offered'

    # ---- offline -------------------------------------------------------------------------------------------------------
    Publish '0.8.2'
    BdfCli $app2 @('update', 'check') | Out-Null
    Stop-Process -Id $server.Id -Force; $server = $null
    Start-Sleep -Seconds 1
    $st = Json (BdfCli $app2 @('update', 'status')).Out
    Check ($st.update_available.version -eq '0.8.2') 'offline: the offer found earlier is still shown'
    $c = BdfCli $app2 @('update', 'check')
    Check ($c.Code -ne 0 -and $c.Out -match 'update server|couldn''t reach' -or $c.Code -ne 0) 'offline: a manual check fails cleanly'
    # R18-1: the ordinary start of the same copy (no test feed configured) shows nothing of the demonstration, and
    # what was downloaded for it cannot be applied.
    $savedFeed = $env:BDF_UPDATE_FEED
    Remove-Item Env:\BDF_UPDATE_FEED
    try {
        $st = Json (BdfCli $app2 @('update', 'status')).Out
        Check ($null -eq $st.update_available -and -not $st.ready_to_install -and $st.check_state -eq 'no_feed') 'reopened without the test feed: no offer, nothing downloadable, and the state says there is no feed'
        $c = BdfCli $app2 @('update', 'apply')
        Check ($c.Code -ne 0 -and (Installed-Version $app2) -eq '0.8.2') 'reopened without the test feed: applying is refused and nothing changes'
    } finally { $env:BDF_UPDATE_FEED = $savedFeed }
    $pdf = Join-Path $Work 'synthetic.pdf'
    $body = '0 0 0 rg 0 0 40 40 re f'
    $obj = @("<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << >> >>", "<< /Length $($body.Length) >>`nstream`n$body`nendstream")
    $txt = "%PDF-1.7`n"; $offs = @(); $i = 1
    foreach ($o in $obj) { $offs += $txt.Length; $txt += "$i 0 obj`n$o`nendobj`n"; $i++ }
    $x = $txt.Length; $txt += "xref`n0 5`n0000000000 65535 f `n"; foreach ($o in $offs) { $txt += ('{0:D10} 00000 n ' -f $o) + "`n" }
    $txt += "trailer << /Size 5 /Root 1 0 R >>`nstartxref`n$x`n%%EOF`n"
    [IO.File]::WriteAllBytes($pdf, [Text.Encoding]::ASCII.GetBytes($txt))
    $r = BdfCli $app2 @('render', $pdf, '--page', '1', '--out', (Join-Path $Work 'p.png'))
    Check ($r.Code -eq 0 -and (Test-Path (Join-Path $Work 'p.png'))) 'offline: opening and rendering a PDF still works'

    # ---- a running program makes the update wait; nothing is ended ------------------------------------------------------------
    $server = Start-Process -FilePath $py -ArgumentList @('-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', $feedRoot) -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    BdfCli $app2 @('update', 'check') | Out-Null
    BdfCli $app2 @('update', 'download') | Out-Null
    # a long-running "job": the MCP server of this copy, alive until its input closes
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = (Join-Path $app2 'bdf-cli.exe'); $psi.Arguments = 'mcp'; $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
    $job = [System.Diagnostics.Process]::Start($psi)
    Start-Sleep -Seconds 1
    BdfCli $app2 @('update', 'apply') | Out-Null
    Start-Sleep -Seconds 6
    Check ((BdfCli $app2 @('--version')).Out -match '0\.8\.0' -and -not $job.HasExited) 'while a CLI job runs, the update waits: nothing replaced, the job is not ended'
    $job.StandardInput.Close()
    $ok = Wait-Until { (Installed-Version $app2) -eq '0.8.2' } 90
    Stop-RestartedBdf $app2
    Check ($ok -and (BdfCli $app2 @('--version')).Out -match '0\.8\.2') 'once the job finished, the update applied'

    # ---- an unmanaged copy and a build without a key ------------------------------------------------------------------------------
    $loose = Join-Path $Work 'loose'; New-Item -ItemType Directory -Path $loose | Out-Null
    Copy-Item (Join-Path $BinRoot '0.8.0\bdf-cli.exe') $loose
    $st = Json (BdfCli $loose @('update', 'status')).Out
    Check (-not $st.managed) 'a copy without its package list says it cannot update itself'
} finally {
    if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Get-Process bdf, bdf-cli, bdf-updater -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Work*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item Env:\BDF_UPDATE_FEED -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($Out) { Set-Content -Path $Out -Value $lines -Encoding utf8 }
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all end-to-end update checks passed'
