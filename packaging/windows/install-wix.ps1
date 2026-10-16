<#
.SYNOPSIS
  Install the WiX Toolset v5 command-line tool (`wix`), checking the package against a pinned hash first.

.DESCRIPTION
  package.ps1 builds the MSI with `wix build`. This installs it as a .NET global tool, but not from
  whatever the package source serves at that moment: the .nupkg is downloaded from nuget.org's
  immutable flat-container URL, its SHA-512 is compared with the value recorded here, and only then is
  it installed from that local file. Updating WiX means changing Version and Sha512 together, after
  reviewing the new release (BUILD-003, finding F07).

  Version 5.0.2: package `wix`, owners wixtoolset / firegiant / robmen, license MS-RL, published
  2024-10-05 (nuget.org registration), repository tag v5.0.2 of github.com/wixtoolset/wix.

.EXAMPLE
  pwsh packaging/windows/install-wix.ps1
#>
param(
  [string] $Version = '5.0.2',
  # SHA-512 of wix.5.0.2.nupkg, base64, as recorded by nuget.org for that version.
  [string] $Sha512 = 'z+fu1deTcaV+TRCo4+6HrJ48Sdnj+yeQjjMStTI3GW7NxFEPaknYCdxrf0qEvgdXrjYdjLnkZKCsNSqTTOo2OA=='
)
$ErrorActionPreference = 'Stop'

if ($Version -ne '5.0.2') { throw "Only WiX 5.0.2 has a recorded hash; update this script together with a new Sha512 ($Version requested)." }

$dir = Join-Path ([IO.Path]::GetTempPath()) ("wix-$Version-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$nupkg = Join-Path $dir "wix.$Version.nupkg"
Invoke-WebRequest -UseBasicParsing -Uri "https://api.nuget.org/v3-flatcontainer/wix/$Version/wix.$Version.nupkg" -OutFile $nupkg

$sha = [System.Security.Cryptography.SHA512]::Create()
$stream = [IO.File]::OpenRead($nupkg)
try { $actual = [Convert]::ToBase64String($sha.ComputeHash($stream)) } finally { $stream.Dispose() }
if ($actual -cne $Sha512) { throw "wix.$Version.nupkg has SHA-512 $actual, expected $Sha512; not installing it." }
Write-Output "ok wix.$Version.nupkg matches the recorded SHA-512"

dotnet tool install --global wix --version $Version --add-source $dir --ignore-failed-sources
if ($LASTEXITCODE -ne 0) { throw "dotnet tool install wix failed with exit code $LASTEXITCODE" }

$tools = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.dotnet\tools'
if ($env:GITHUB_PATH) { Add-Content -Path $env:GITHUB_PATH -Value $tools }
$env:PATH = "$tools;$env:PATH"
wix --version
if ($LASTEXITCODE -ne 0) { throw "wix --version failed with exit code $LASTEXITCODE" }
