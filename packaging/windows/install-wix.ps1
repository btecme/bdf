<#
.SYNOPSIS
  Install the WiX Toolset v5 command-line tool (`wix`) from a package whose hash was checked first, and
  check afterwards that the installed bytes are those bytes.

.DESCRIPTION
  package.ps1 builds the MSI with `wix build`. The .nupkg is downloaded from nuget.org's immutable
  flat-container URL, its SHA-512 is compared with the value recorded here, and only then is it installed,
  from a NuGet configuration that names no other source:

    * `--configfile` makes `dotnet tool install` use only that file: the `<clear />` in it drops the user,
      machine and repository-level feeds, and the verified folder is the only source, so another feed
      cannot supply a `wix` 5.0.2 (`--add-source` would only have added a feed to the others, and NuGet
      queries feeds in parallel);
    * the global-packages folder and the HTTP cache are fresh directories for this install, so a copy
      cached earlier cannot be used instead;
    * the tool goes to a fresh `--tool-path`, not the machine's global tool folder, so a `wix` already
      installed there is neither reused nor run.

  After the install, the package hash NuGet recorded for what it unpacked (`wix.<version>.nupkg.sha512`)
  must equal the recorded value; if it is missing or differs, the install is refused. `wix` is then run by
  its full path from the new tool folder, which is also put first on PATH for later steps.

  Updating WiX means changing Version and Sha512 together after reviewing the new release (BUILD-003,
  finding F07). Version 5.0.2: package `wix`, owners wixtoolset / firegiant / robmen, license MS-RL,
  published 2024-10-05 (nuget.org registration), repository tag v5.0.2 of github.com/wixtoolset/wix.
  test-install-wix.ps1 exercises the refusals and the isolation.

.PARAMETER Work
  Working directory for the feed, configuration, caches and tool folder (default: a new temporary folder).

.PARAMETER Nupkg
  Where to get wix.<version>.nupkg: a local file, or an https URL (default: nuget.org's flat container).
  The hash is checked whatever the source.

.EXAMPLE
  pwsh packaging/windows/install-wix.ps1
#>
param(
  [string] $Version = '5.0.2',
  # SHA-512 of wix.5.0.2.nupkg, base64, as recorded by nuget.org for that version.
  [string] $Sha512 = 'z+fu1deTcaV+TRCo4+6HrJ48Sdnj+yeQjjMStTI3GW7NxFEPaknYCdxrf0qEvgdXrjYdjLnkZKCsNSqTTOo2OA==',
  [string] $Work = '',
  [string] $Nupkg = ''
)
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version '$Version' is not a plain x.y.z version." }
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetTempPath()) ("wix-$Version-" + [Guid]::NewGuid().ToString('N')) }
if (-not $Nupkg) { $Nupkg = "https://api.nuget.org/v3-flatcontainer/wix/$Version/wix.$Version.nupkg" }

$feed = Join-Path $Work 'feed'
$packages = Join-Path $Work 'packages'
$httpCache = Join-Path $Work 'http-cache'
$tools = Join-Path $Work 'tools'
$config = Join-Path $Work 'nuget.config'
foreach ($d in $feed, $packages, $httpCache) { New-Item -ItemType Directory -Force -Path $d | Out-Null }

$file = Join-Path $feed "wix.$Version.nupkg"
if (Test-Path -LiteralPath $Nupkg) { Copy-Item -LiteralPath $Nupkg -Destination $file }
else { Invoke-WebRequest -UseBasicParsing -Uri $Nupkg -OutFile $file }

function Get-Sha512Base64([string] $Path) {
  $sha = [System.Security.Cryptography.SHA512]::Create()
  $stream = [IO.File]::OpenRead($Path)
  try { [Convert]::ToBase64String($sha.ComputeHash($stream)) } finally { $stream.Dispose() }
}

$actual = Get-Sha512Base64 $file
if ($actual -cne $Sha512) {
  Remove-Item -LiteralPath $file -Force
  throw "wix.$Version.nupkg has SHA-512 $actual, expected $Sha512; not installing it."
}
Write-Output "ok wix.$Version.nupkg matches the recorded SHA-512"

# The only package source is the folder holding the verified file.
$xml = @"
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="verified" value="$([Security.SecurityElement]::Escape($feed))" />
  </packageSources>
</configuration>
"@
[IO.File]::WriteAllText($config, $xml, (New-Object Text.UTF8Encoding($false)))

$saved = @{ NUGET_PACKAGES = $env:NUGET_PACKAGES; NUGET_HTTP_CACHE_PATH = $env:NUGET_HTTP_CACHE_PATH }
$env:NUGET_PACKAGES = $packages
$env:NUGET_HTTP_CACHE_PATH = $httpCache
try {
  dotnet tool install wix --version $Version --tool-path $tools --configfile $config
  if ($LASTEXITCODE -ne 0) { throw "dotnet tool install wix failed with exit code $LASTEXITCODE" }
} finally {
  $env:NUGET_PACKAGES = $saved.NUGET_PACKAGES
  $env:NUGET_HTTP_CACHE_PATH = $saved.NUGET_HTTP_CACHE_PATH
}

# What NuGet unpacked must be the verified package: it records the hash of the package it installed.
$recorded = @(Get-ChildItem -LiteralPath $tools, $packages -Recurse -Filter "wix.$Version.nupkg.sha512" -File -ErrorAction SilentlyContinue)
if ($recorded.Count -eq 0) { throw "no wix.$Version.nupkg.sha512 under ${tools} or ${packages}: cannot confirm what was installed; refusing." }
foreach ($r in $recorded) {
  $value = (Get-Content -LiteralPath $r.FullName -Raw).Trim()
  if ($value -cne $Sha512) { throw "the installed package ($($r.FullName)) has SHA-512 $value, not the verified $Sha512; refusing." }
}
Write-Output "ok the installed package is the verified one ($($recorded.Count) record(s))"

$wix = Join-Path $tools 'wix.exe'
if (-not (Test-Path -LiteralPath $wix)) { throw "wix.exe is not in $tools after the install." }
if ($env:GITHUB_PATH) { Add-Content -Path $env:GITHUB_PATH -Value $tools }
$env:PATH = "$tools;$env:PATH"
& $wix --version
if ($LASTEXITCODE -ne 0) { throw "wix --version failed with exit code $LASTEXITCODE" }
