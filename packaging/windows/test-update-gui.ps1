<#
.SYNOPSIS
  Drives the real BDF window through a complete update: Later, Download, a cancelled close, the unsaved-work question, the update,
  the restart, and the person's data afterwards.
.DESCRIPTION
  Everything runs on a throwaway copy of a development build (it trusts a throwaway test key) against a local test feed on
  127.0.0.1, so nothing of the person's is involved. The window is driven through its own control channel (the same one the
  automated UI tests and `bdf-cli ui` use): widgets are found and clicked by their accessible labels, as an assistive technology would.
  Steps, each checked:
    1. the green button appears and says it is a test offer; Later closes the dialog and changes nothing;
    2. Download update fetches and verifies the package; BDF is still running the old version;
    3. a document with an unsaved change is open; "Close BDF and update" asks about the unsaved work; Cancel withdraws the update:
       BDF is still running, the document is still open and still unsaved, nothing was replaced, no updater is running;
    4. the same again, and this time Don't save: BDF closes, the helper replaces the program, BDF starts again as the new version
       from the same folder, with the document's file open;
    5. BDFData (a settings file, a recovery file, a marker file) is byte-for-byte what it was, BDFPrevious holds the old version.
  Needs a window that is drawn (a desktop session). Touches only a temp folder and the current user's control-channel file.
    powershell -File packaging\windows\test-update-gui.ps1 -BdfDir dist\demo\BDF -FeedDir dist\demo\feed [-EvidenceDir out]
#>
param(
    [Parameter(Mandatory)][string]$BdfDir,
    [Parameter(Mandatory)][string]$FeedDir,
    [string]$FeedScript,
    [int]$Port = 8089,
    [string]$EvidenceDir,
    [string]$FromVersion = '0.8.0',
    [string]$ToVersion = '0.8.1'
)
$ErrorActionPreference = 'Stop'
$BdfDir = (Resolve-Path $BdfDir).Path
$FeedDir = (Resolve-Path $FeedDir).Path
if (-not $FeedScript) { $FeedScript = Join-Path $PSScriptRoot 'preview-feed.ps1' }
if (-not (Test-Path $FeedScript)) { $FeedScript = Join-Path (Split-Path $FeedDir -Parent) 'preview-feed.ps1' }
$Work = Join-Path ([IO.Path]::GetTempPath()) ('bdf-gui-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$App = Join-Path $Work 'BDF'
$Ctl = Join-Path $env:USERPROFILE ('.bdf-control-gui-' + [guid]::NewGuid().ToString('N').Substring(0, 6) + '.json')
if ($EvidenceDir) { New-Item -ItemType Directory -Path $EvidenceDir -Force | Out-Null; $EvidenceDir = (Resolve-Path $EvidenceDir).Path }
$failures = 0
function Check([bool]$ok, [string]$what) { if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ } }
function Version-Of([string]$dir) {
    $m = (Get-Content (Join-Path $dir 'bdf-package.json') -Raw -ErrorAction SilentlyContinue) | Select-String '"version": "([0-9.]+)"'
    if ($m) { $m.Matches[0].Groups[1].Value } else { '' }
}
function BdfCli([string[]]$a) {
    $previous = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $o = & (Join-Path $App 'bdf-cli.exe') @a 2>&1 | Out-String; $code = $LASTEXITCODE } finally { $ErrorActionPreference = $previous }
    [pscustomobject]@{ Code = $code; Out = $o }
}
function Ui([string[]]$a) { $r = BdfCli (@('ui', '--control', $Ctl) + $a); if ($r.Code -ne 0) { return $null }; try { $r.Out | ConvertFrom-Json } catch { $null } }
function Find-Widget([string]$Label) {
    $r = Ui @('inspect', "query=$Label")
    if (-not $r) { return $null }
    @($r.widgets | Where-Object { $_.label -ieq $Label -and $_.enabled }) | Select-Object -First 1
}
function Wait-Widget([string]$Label, [int]$Seconds = 40) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { $w = Find-Widget $Label; if ($w) { return $w }; Start-Sleep -Milliseconds 400 }
    $null
}
function Wait-Gone([string]$Label, [int]$Seconds = 15) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (-not (Find-Widget $Label)) { return $true }; Start-Sleep -Milliseconds 300 }
    $false
}
function Click([string]$Label) { $r = Ui @('click', "label=$Label"); [bool]$r }
function Shot([string]$Name) { if ($EvidenceDir) { [void](BdfCli @('ui', '--control', $Ctl, 'screenshot', '--out', (Join-Path $EvidenceDir "$Name.png"))) } }
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 60) {
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 400 }
    $false
}
function Mine { Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($App, [StringComparison]::OrdinalIgnoreCase) } }
# A one-page PDF with a correct cross-reference table, made here so the test needs no file from anyone.
function New-TestPdf([string]$Path) {
    $objs = @(
        '<< /Type /Catalog /Pages 2 0 R >>',
        '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
        $null,
        '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
    )
    $stream = "BT /F1 18 Tf 30 100 Td (BDF update qualification) Tj ET"
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append("%PDF-1.4`n")
    $offsets = @()
    for ($i = 0; $i -lt $objs.Count; $i++) {
        $offsets += $sb.Length
        $body = if ($i -eq 3) { "<< /Length $($stream.Length) >>`nstream`n$stream`nendstream" } else { $objs[$i] }
        [void]$sb.Append("$($i + 1) 0 obj`n$body`nendobj`n")
    }
    $xref = $sb.Length
    [void]$sb.Append("xref`n0 $($objs.Count + 1)`n0000000000 65535 f `n")
    foreach ($o in $offsets) { [void]$sb.Append(('{0:0000000000} 00000 n `n' -f $o)) }
    [void]$sb.Append("trailer`n<< /Size $($objs.Count + 1) /Root 1 0 R >>`nstartxref`n$xref`n%%EOF`n")
    [IO.File]::WriteAllBytes($Path, [Text.Encoding]::ASCII.GetBytes($sb.ToString()))
}

$feed = $null; $bdf = $null
try {
    New-Item -ItemType Directory -Path $Work | Out-Null
    Copy-Item -LiteralPath $BdfDir -Destination $App -Recurse
    $pdf = Join-Path $Work 'qual.pdf'
    New-TestPdf $pdf
    $pdfHash = (Get-FileHash $pdf).Hash
    # the person's data
    $data = Join-Path $App 'BDFData'
    New-Item -ItemType Directory -Path "$data\recovery" -Force | Out-Null
    Set-Content "$data\qual-marker.txt" 'marker the update must not touch'
    Set-Content "$data\recovery\other.autosave" 'unsaved work from an earlier session'

    $feed = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$FeedScript`"", "`"$FeedDir`"", "$Port") -PassThru -WindowStyle Hidden
    $ready = Wait-Until { try { (Invoke-WebRequest -Uri "http://127.0.0.1:$Port/release.json" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 } catch { $false } } 20
    Check $ready "the local test feed answers on 127.0.0.1:$Port"
    if (-not $ready) { throw 'no feed' }

    Remove-Item $Ctl -ErrorAction SilentlyContinue
    $env:BDF_UPDATE_FEED = "http://127.0.0.1:$Port/release.json"
    try { $bdf = Start-Process -FilePath (Join-Path $App 'bdf.exe') -ArgumentList @('--control', "`"$Ctl`"") -PassThru } finally { Remove-Item Env:\BDF_UPDATE_FEED -ErrorAction SilentlyContinue }
    Check (Wait-Until { Test-Path $Ctl } 30) 'BDF started and opened its control channel'

    $offer = 'Test offer from a development feed. This is not a BDF release.'
    $btn = Wait-Widget $offer 60
    Check ([bool]$btn) 'the test offer button appears, and its accessible name says it is a test offer, not a release'
    Check ((Version-Of $App) -eq $FromVersion) "the copy is $FromVersion before anything"

    # 1. Later
    [void](Click $offer)
    Check ([bool](Wait-Widget 'Download update' 15)) 'the dialog opens with Download update'
    Shot '1-dialog'
    Check (Click 'Later') 'Later is clickable'
    Check (Wait-Gone 'Download update' 10) 'Later closes the dialog'
    Check ((Version-Of $App) -eq $FromVersion -and -not (Test-Path (Join-Path $App 'BDFUpdate\stage'))) 'Later changed nothing'
    Check ([bool](Find-Widget $offer)) 'the offer is still there after Later'

    # 2. Download
    [void](Click $offer)
    [void](Wait-Widget 'Download update' 15)
    Check (Click 'Download update') 'Download update is clickable'
    $close = Wait-Widget 'Close BDF and update' 90
    Check ([bool]$close) 'the package is downloaded and verified: Close BDF and update is offered'
    Shot '2-ready'
    Check ((Version-Of $App) -eq $FromVersion) 'downloading did not replace anything'

    # 3. a document with an unsaved change; Close BDF and update; Cancel
    [void](Ui @('open', "path=$pdf"))
    Check (Wait-Until { $s = Ui @('state'); $s -and @($s.documents).Count -eq 1 } 30) 'a document is open'
    [void](Ui @('command', 'id=page.rotate'))
    Check (Wait-Until { $s = Ui @('state'); $s -and $s.documents[0].dirty } 15) 'the document has an unsaved change'
    Check (Click 'Close BDF and update') 'Close BDF and update is clickable'
    Check (Wait-Until { $s = Ui @('state'); $s -and $s.close_prompt } 20) 'BDF asks about the unsaved work before closing'
    Shot '3-unsaved-question'
    Check ([bool](Wait-Widget "Don't save" 10)) "the question offers Save, Don't save and Cancel"
    Check (Click 'Cancel') 'Cancel is clickable'
    Check (Wait-Until { $s = Ui @('state'); $s -and -not $s.close_prompt } 15) 'Cancel closes the question'
    Start-Sleep -Seconds 3
    $s = Ui @('state')
    Check ($bdf -and -not $bdf.HasExited -and $s -and @($s.documents).Count -eq 1 -and $s.documents[0].dirty) 'after Cancel BDF is still running and the document is still open and still unsaved'
    Check ((Version-Of $App) -eq $FromVersion -and @(Mine | Where-Object { $_.ProcessName -eq 'bdf-updater' }).Count -eq 0) 'after Cancel nothing was replaced and no updater is running'
    Shot '3b-after-cancel'

    # 4. the same again, this time Don't save
    $keep = @((Join-Path $data 'qual-marker.txt'), (Join-Path $data 'recovery\other.autosave'))
    $dataBefore = @{}; foreach ($k in $keep) { $dataBefore[$k] = (Get-FileHash -LiteralPath $k).Hash }
    $startedAfter = Get-Date
    $btn = Wait-Widget $offer 15
    if ($btn) { [void](Click $offer) }
    Check ([bool](Wait-Widget 'Close BDF and update' 20)) 'the update is still on offer after the cancelled close'
    [void](Click 'Close BDF and update')
    Check (Wait-Until { $s = Ui @('state'); $s -and $s.close_prompt } 20) 'the question comes again'
    Check (Click "Don't save") "Don't save is clickable"
    Check (Wait-Until { (Version-Of $App) -eq $ToVersion } 120) "BDF closed and the helper replaced the program: the copy is now $ToVersion"
    Check (Wait-Until { [bool](Mine | Where-Object { $_.ProcessName -eq 'bdf' -and $_.StartTime -gt $startedAfter }) } 60) 'BDF started again from the same folder'
    $new = Mine | Where-Object { $_.ProcessName -eq 'bdf' -and $_.StartTime -gt $startedAfter } | Select-Object -First 1
    Start-Sleep -Seconds 6
    $title = if ($new) { ($new | ForEach-Object { $_.Refresh(); $_.MainWindowTitle }) } else { '' }
    Check ($title -match 'qual\.pdf') "the document's file was reopened (window title: $title)"
    Check ((BdfCli @('update', 'status')).Out -match "`"current`": `"$([regex]::Escape($ToVersion))`"") "the new BDF reports version $ToVersion"

    # 5. the person's data
    $same = $true
    foreach ($k in $keep) { if (-not (Test-Path -LiteralPath $k) -or (Get-FileHash -LiteralPath $k).Hash -ne $dataBefore[$k]) { $same = $false } }
    Check $same 'BDFData: the marker and the recovery file are byte-for-byte what they were'
    $prevVersion = Version-Of (Join-Path $App 'BDFPrevious')
    Check ($prevVersion -eq $FromVersion) "BDFPrevious holds the version the person had ($prevVersion)"
    Check ((Get-FileHash $pdf).Hash -eq $pdfHash) "the document's file on disk was not saved over (Don't save)"
} finally {
    Mine | Where-Object { $_.ProcessName -match '^bdf' } | Stop-Process -Force -ErrorAction SilentlyContinue
    if ($feed) { Stop-Process -Id $feed.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800
    Remove-Item $Ctl -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all GUI update checks passed'
# Explicit: a native command above may have left $LASTEXITCODE non-zero, and a CI shell would report that instead.
exit 0
