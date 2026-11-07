<#
.SYNOPSIS
  Shows the update flow with a development build and a local test feed, on a throwaway copy (BUILD-018).

.DESCRIPTION
  The ordinary preview (a plain extracted package) never shows an update. This is the separate, explicit demonstration:

    1. copies the development build in -DemoRoot\BDF to a fresh temporary folder, so the build you keep is never touched
       and nothing from the demonstration survives it;
    2. serves -DemoRoot\feed on http://127.0.0.1:<port>/ (this machine only) with the test release in it;
    3. starts the copy with BDF_UPDATE_FEED pointing at that feed, for that process only (the variable is never set for
       your session, your account or the machine);
    4. when you close BDF, stops the server and deletes the temporary copy (unless -Keep).

  Everything is labelled a test: the button reads "Test update", the dialog says the offer is not a BDF release, and the
  release notes are one synthetic line. The development build trusts a throwaway test key and only reads a feed from a
  loopback address or HTTPS. Nothing is sent anywhere.

.PARAMETER DemoRoot  The demo folder: BDF\ (the development build), feed\ (release.json and the test package).
.PARAMETER Port      The loopback port the feed was signed for (the package address inside release.json names it). Default 8089.
.PARAMETER Keep      Keep the temporary copy (and its BDFData) after BDF closes, and print where it is.
#>
param(
    [string]$DemoRoot = $PSScriptRoot,
    [int]$Port = 8089,
    [switch]$Keep
)
$ErrorActionPreference = 'Stop'
$DemoRoot = (Resolve-Path -LiteralPath $DemoRoot).Path
$source = Join-Path $DemoRoot 'BDF'
$feed = Join-Path $DemoRoot 'feed'
foreach ($p in @((Join-Path $source 'bdf.exe'), (Join-Path $feed 'release.json'))) {
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "not found: $p (is -DemoRoot the demo folder?)" }
}
$server = Join-Path $PSScriptRoot 'preview-feed.ps1'
if (-not (Test-Path -LiteralPath $server)) { $server = Join-Path $DemoRoot 'preview-feed.ps1' }

$work = Join-Path ([IO.Path]::GetTempPath()) ('BDF-update-demo-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$copy = Join-Path $work 'BDF'
New-Item -ItemType Directory -Path $work | Out-Null
Copy-Item -LiteralPath $source -Destination $copy -Recurse
Write-Output "Demonstration copy: $copy"

$job = Start-Job -FilePath $server -ArgumentList $feed, $Port
try {
    $ready = $false
    for ($i = 0; $i -lt 50 -and -not $ready; $i++) {
        Start-Sleep -Milliseconds 100
        if ($job.State -eq 'Failed' -or $job.State -eq 'Completed') { break }
        try { $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/release.json" -UseBasicParsing -TimeoutSec 2; $ready = ($r.StatusCode -eq 200) } catch { }
    }
    if (-not $ready) { throw "the test feed did not start on port $Port (is something else using it?): $(Receive-Job $job -ErrorAction SilentlyContinue | Out-String)" }
    Write-Output "Test feed: http://127.0.0.1:$Port/release.json. Starting BDF; a few seconds in, the green 'Test update' button appears. Close BDF to finish."
    $env:BDF_UPDATE_FEED = "http://127.0.0.1:$Port/release.json"
    try { $process = Start-Process -FilePath (Join-Path $copy 'bdf.exe') -PassThru } finally { Remove-Item Env:\BDF_UPDATE_FEED -ErrorAction SilentlyContinue }
    # An update restarts BDF from the same folder as a new process: wait until nothing runs from the copy any more.
    $process.WaitForExit()
    do {
        Start-Sleep -Seconds 2
        $running = @(Get-Process -Name 'bdf', 'bdf-cli', 'bdf-worker', 'bdf-updater' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($copy, [StringComparison]::OrdinalIgnoreCase) })
    } while ($running.Count -gt 0)
} finally {
    Stop-Job $job -ErrorAction SilentlyContinue
    Remove-Job $job -Force -ErrorAction SilentlyContinue
    if ($Keep) {
        Write-Output "Kept: $copy"
    } else {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        Write-Output 'The demonstration copy and its data were removed. The preview you keep was not touched.'
    }
}
