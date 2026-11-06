<#
.SYNOPSIS
  Deploys the BDF portable package into each user's own profile (Action1 / any Local System deployment tool).

.DESCRIPTION
  Runs as Local System. It never starts BDF, never runs anything from the package or from a user's folder,
  and never touches a user's BDFData folder. What it does:

    1. Verifies the release exactly as the in-app updater does: the release file (bdf-update.json) is an
       envelope whose signature (ECDSA P-256 over SHA-256) is checked against -PublicKey, then the package's
       size and SHA-256 against that signed information, then every file in the ZIP against the package's own
       list. Nothing is read from an unauthenticated source. Unsigned or tampered input stops the run.
    2. For each intended user profile, installs (or updates) the portable copy in
       <profile>\AppData\Local\Programs\BDF, by renaming files, with the previous version kept in
       BDFPrevious and a rollback if anything fails. A copy that is running, or any other open file, is
       never replaced: that user is skipped and picked up by the next run.
    3. Gives the user ownership and Modify rights on their own copy so BDF can update itself without
       administrator rights. No other permission is changed.

  New users: -IncludeDefaultProfile also installs into C:\Users\Default, which Windows copies into every
  profile created later. Users who already exist are covered by running this script again (Action1 policy).

  Not done on purpose: no change to default apps or any registry (each person registers BDF in
  Preferences > Windows integration, if they want to), no shortcut, no firewall or service.

.PARAMETER PackageZip   The portable ZIP (bdf-<version>-windows-x64-portable.zip).
.PARAMETER ReleaseFile  The signed release file (bdf-update.json) published with that ZIP.
.PARAMETER PublicKey    The release signing public key: 130 hex characters (uncompressed P-256 point).
.PARAMETER Users        Only these user names (as in C:\Users\<name>). Default: every real user profile.
.PARAMETER IncludeDefaultProfile  Also install for users created later.
.PARAMETER Channel      stable (default) or dev (test releases signed with a test key).
.PARAMETER UsersRoot    Test hook: treat the subfolders of this folder as the profiles (no ownership changes).
.PARAMETER WhatIf       Verify and report what would happen without changing anything.

.NOTES
  Exit codes: 0 done (or nothing to do; skipped users are listed and retried by the next run),
              1 verification or a deployment failed, 2 bad parameters.
  Windows PowerShell 5.1 and PowerShell 7. Log: C:\ProgramData\BDF\deploy.log
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackageZip,
    [Parameter(Mandatory)][string]$ReleaseFile,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{130}$')][string]$PublicKey,
    [string[]]$Users,
    [switch]$IncludeDefaultProfile,
    [ValidateSet('stable', 'dev')][string]$Channel = 'stable',
    [string]$UsersRoot,
    [switch]$WhatIf
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# `-File` passes "a,b" as one string: accept a comma-separated list as well as an array.
if ($Users) { $Users = @($Users | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }

$AppSub = 'AppData\Local\Programs\BDF'
$PackageFile = 'bdf-package.json'
$Reserved = @('bdfdata', 'bdfupdate', 'bdfprevious')
$LogDir = Join-Path $env:ProgramData 'BDF'
$Skipped = New-Object System.Collections.ArrayList
$Failed = New-Object System.Collections.ArrayList
$Done = New-Object System.Collections.ArrayList

function Write-Log([string]$Message) {
    $line = '{0:u} {1}' -f (Get-Date), $Message
    Write-Output $line
    try {
        if (-not (Test-Path $LogDir)) { New-Item -ItemType Directory -Path $LogDir -Force | Out-Null }
        Add-Content -Path (Join-Path $LogDir 'deploy.log') -Value $line -Encoding UTF8
    } catch { }
}

function Stop-Run([string]$Message, [int]$Code = 1) {
    Write-Log "STOP: $Message"
    exit $Code
}

function ConvertFrom-Hex([string]$Hex) {
    $bytes = New-Object byte[] ($Hex.Length / 2)
    for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($Hex.Substring(2 * $i, 2), 16) }
    , $bytes
}

function Get-Sha256Hex([System.IO.Stream]$Stream) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($Stream)) -replace '-', '').ToLowerInvariant() } finally { $sha.Dispose() }
}

function Test-SafeRelativePath([string]$Path) {
    if ([string]::IsNullOrEmpty($Path) -or $Path.Length -gt 200) { return $false }
    $parts = $Path.Split('/')
    if ($Reserved -contains $parts[0].ToLowerInvariant()) { return $false }
    foreach ($p in $parts) {
        if ($p -eq '' -or $p -eq '.' -or $p -eq '..' -or $p.Length -gt 100) { return $false }
        if ($p.EndsWith('.') -or $p.EndsWith(' ') -or $p.StartsWith(' ')) { return $false }
        if ($p -match '[\\:*?"<>|\x00-\x1f]') { return $false }
        if ($p.Split('.')[0] -match '^(?i:CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])$') { return $false }
    }
    $true
}

# ---- 1. verify the release ---------------------------------------------------------------------------------

foreach ($f in @($PackageZip, $ReleaseFile)) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { Stop-Run "not found: $f" 2 }
}

try {
    $envelope = Get-Content -LiteralPath $ReleaseFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $payload = [Convert]::FromBase64String([string]$envelope.payload)
    $signature = ConvertFrom-Hex ([string]$envelope.signature).Trim()
} catch { Stop-Run "the release file is not a BDF release envelope ($($_.Exception.Message))" }
if ($payload.Length -gt 262144 -or $signature.Length -ne 64) { Stop-Run 'the release file is malformed' }

$key = ConvertFrom-Hex $PublicKey
if ($key[0] -ne 4) { Stop-Run '-PublicKey must be an uncompressed P-256 point (starts with 04)' 2 }
$ecParams = New-Object System.Security.Cryptography.ECParameters
$ecParams.Curve = [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
# ECParameters and ECPoint are value types: build the point whole and assign it.
$point = New-Object System.Security.Cryptography.ECPoint
$point.X = [byte[]]$key[1..32]
$point.Y = [byte[]]$key[33..64]
$ecParams.Q = $point
$ecdsa = [System.Security.Cryptography.ECDsa]::Create($ecParams)
if (-not $ecdsa.VerifyData($payload, $signature, [System.Security.Cryptography.HashAlgorithmName]::SHA256)) {
    Stop-Run 'the release signature does not match -PublicKey: this release is not trusted and nothing was installed'
}
$manifest = [System.Text.Encoding]::UTF8.GetString($payload) | ConvertFrom-Json
if ($manifest.schema -ne 1 -or $manifest.product -ne 'bdf') { Stop-Run 'the release information is not for BDF' }
if ($manifest.channel -ne $Channel) { Stop-Run "the release is on the '$($manifest.channel)' channel, this run is for '$Channel'" }
if ([string]$manifest.version -notmatch '^\d+\.\d+\.\d+$') { Stop-Run 'the release version is not MAJOR.MINOR.PATCH' }
$version = [version]$manifest.version
$entry = @($manifest.packages | Where-Object { $_.os -eq 'windows' -and $_.arch -eq 'x64' -and $_.kind -eq 'portable-zip' }) | Select-Object -First 1
if (-not $entry) { Stop-Run 'the release has no Windows x64 portable package' }

$zipInfo = Get-Item -LiteralPath $PackageZip
if ($zipInfo.Length -ne [int64]$entry.size) { Stop-Run "the package is $($zipInfo.Length) bytes, the signed release says $($entry.size)" }
$stream = [System.IO.File]::OpenRead($zipInfo.FullName)
try { $zipHash = Get-Sha256Hex $stream } finally { $stream.Dispose() }
if ($zipHash -ne ([string]$entry.sha256).ToLowerInvariant()) { Stop-Run 'the package does not match the signed release (hash differs)' }

# The package against its own list: one top folder, safe names, every listed file exactly, nothing else.
$archive = [System.IO.Compression.ZipFile]::OpenRead($zipInfo.FullName)
try {
    $top = $null
    $files = @{}
    foreach ($e in $archive.Entries) {
        if ($e.FullName.Contains('\')) { Stop-Run "unsafe path in the package: $($e.FullName)" }
        $name = $e.FullName.TrimEnd('/')
        $first, $rest = $name -split '/', 2
        if (-not (Test-SafeRelativePath $first)) { Stop-Run "unsafe path in the package: $($e.FullName)" }
        if ($null -eq $top) { $top = $first } elseif ($top -ne $first) { Stop-Run 'the package has more than one top-level folder' }
        if ($e.FullName.EndsWith('/')) { continue }
        if (-not $rest) { Stop-Run 'the package has a file outside its folder' }
        if ($rest -ne $PackageFile -and -not (Test-SafeRelativePath $rest)) { Stop-Run "unsafe path in the package: $($e.FullName)" }
        $key2 = $rest.ToLowerInvariant()
        if ($files.ContainsKey($key2)) { Stop-Run "duplicate entry in the package: $rest" }
        $files[$key2] = $e
    }
    if (-not $files.ContainsKey($PackageFile)) { Stop-Run "the package has no $PackageFile" }
    $infoEntry = $files[$PackageFile]
    if ($infoEntry.Length -gt 1048576) { Stop-Run 'the package list is too large' }
    $reader = New-Object System.IO.StreamReader($infoEntry.Open(), [System.Text.Encoding]::UTF8)
    try { $infoText = $reader.ReadToEnd() } finally { $reader.Dispose() }
    $infoBytes = [System.Text.Encoding]::UTF8.GetBytes($infoText)
    $info = $infoText | ConvertFrom-Json
    if ($info.schema -ne 1 -or $info.product -ne 'bdf' -or $info.os -ne 'windows' -or $info.arch -ne 'x64' -or $info.version -ne [string]$manifest.version) {
        Stop-Run 'the package list does not describe this release'
    }
    $listed = @($info.files)
    if ($listed.Count -eq 0 -or $listed.Count -gt 2000) { Stop-Run 'the package lists an impossible number of files' }
    if ($files.Count -ne $listed.Count + 1) { Stop-Run 'the package contains files its list does not name' }
    foreach ($f in $listed) {
        if (-not (Test-SafeRelativePath $f.path) -or $f.path -eq $PackageFile) { Stop-Run "the package lists an unsafe path: $($f.path)" }
        $e = $files[([string]$f.path).ToLowerInvariant()]
        if ($null -eq $e) { Stop-Run "the package is missing $($f.path)" }
        if ($e.Length -ne [int64]$f.size) { Stop-Run "$($f.path) has a different size than the package list says" }
        $s = $e.Open()
        try { $h = Get-Sha256Hex $s } finally { $s.Dispose() }
        if ($h -ne ([string]$f.sha256).ToLowerInvariant()) { Stop-Run "$($f.path) does not match the package list" }
    }
} finally { $archive.Dispose() }
Write-Log "verified BDF $version (signature, package hash, $($listed.Count) files)"

# ---- 2. the intended profiles ------------------------------------------------------------------------------

function Get-TargetProfiles {
    $list = New-Object System.Collections.ArrayList
    if ($UsersRoot) {
        foreach ($d in Get-ChildItem -LiteralPath $UsersRoot -Directory) { [void]$list.Add([pscustomobject]@{ Name = $d.Name; Path = $d.FullName; Sid = $null }) }
    } else {
        $base = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList'
        foreach ($k in Get-ChildItem $base) {
            $sid = $k.PSChildName
            # Real people only: local, domain and Azure AD accounts. Not SYSTEM, LOCAL SERVICE, NETWORK SERVICE or service accounts.
            if ($sid -notmatch '^S-1-5-21-' -and $sid -notmatch '^S-1-12-1-') { continue }
            $path = [string]((Get-ItemProperty $k.PSPath).ProfileImagePath)
            if (-not $path -or -not (Test-Path -LiteralPath (Join-Path $path 'NTUSER.DAT'))) { continue }
            [void]$list.Add([pscustomobject]@{ Name = (Split-Path $path -Leaf); Path = $path; Sid = $sid })
        }
    }
    if ($Users) { $list = @($list | Where-Object { $Users -contains $_.Name }) }
    if ($IncludeDefaultProfile) {
        $default = if ($UsersRoot) { Join-Path $UsersRoot 'Default' } else { Join-Path $env:SystemDrive 'Users\Default' }
        if (Test-Path -LiteralPath $default) { $list = @($list) + [pscustomobject]@{ Name = 'Default'; Path = $default; Sid = $null } }
    }
    @($list)
}

function Test-NoReparse([string]$Root, [string]$Dir) {
    # SYSTEM must never write through a junction or symbolic link a user could have planted.
    $p = $Dir
    while ($p -and $p.Length -ge $Root.Length) {
        if (Test-Path -LiteralPath $p) {
            if ((Get-Item -LiteralPath $p -Force).Attributes -band [System.IO.FileAttributes]::ReparsePoint) { return $false }
        }
        $p = Split-Path $p -Parent
    }
    $true
}

function Test-InUse([string]$File) {
    if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { return $false }
    try {
        $fs = [System.IO.File]::Open($File, 'Open', 'ReadWrite', 'None')
        $fs.Dispose()
        $false
    } catch [System.IO.IOException] { $true } catch { $false }
}

function Install-ForProfile($profile) {
    $app = Join-Path $profile.Path $AppSub
    if (-not (Test-NoReparse $profile.Path $app)) { throw 'a link or junction is in the way of the BDF folder; refusing to write through it' }
    $installedVersion = $null
    $oldFiles = @()
    $oldListPath = Join-Path $app $PackageFile
    if (Test-Path -LiteralPath $app) {
        if (-not (Test-Path -LiteralPath $oldListPath)) { throw "a folder exists at $app that is not a managed BDF copy; leaving it alone" }
        $oldInfo = Get-Content -LiteralPath $oldListPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $installedVersion = [version]$oldInfo.version
        $oldFiles = @($oldInfo.files | ForEach-Object { [string]$_.path })
        if ($installedVersion -ge $version) { return 'current' }
    }
    if ($WhatIf) { return $(if ($installedVersion) { "would update $installedVersion -> $version" } else { "would install $version" }) }

    # Files to be replaced must be free: a running BDF (or its CLI, worker or updater) is never replaced or ended.
    $union = @($oldFiles + @($listed | ForEach-Object { [string]$_.path }) + $PackageFile) | Select-Object -Unique
    foreach ($rel in $union) {
        $live = Join-Path $app ($rel -replace '/', '\')
        if (Test-InUse $live) { return 'in-use' }
    }
    New-Item -ItemType Directory -Path $app -Force | Out-Null
    $stage = Join-Path $app 'BDFUpdate\deploy-stage'
    $prev = Join-Path $app 'BDFUpdate\deploy-previous'
    foreach ($d in @($stage, $prev)) { if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force } }
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    New-Item -ItemType Directory -Path $prev -Force | Out-Null

    $moved = New-Object System.Collections.ArrayList   # @(live, prev) pairs displaced, for rollback
    $placed = New-Object System.Collections.ArrayList  # live paths of new files put in place
    $zip = [System.IO.Compression.ZipFile]::OpenRead($zipInfo.FullName)
    try {
        # Stage every file, hashing as it is written.
        $all = @($listed) + [pscustomobject]@{ path = $PackageFile; size = $infoBytes.Length; sha256 = $null }
        foreach ($f in $all) {
            $rel = [string]$f.path
            $target = Join-Path $stage ($rel -replace '/', '\')
            New-Item -ItemType Directory -Path (Split-Path $target -Parent) -Force | Out-Null
            $entry = $zip.GetEntry("$top/$rel")
            $in = $entry.Open()
            $out = [System.IO.File]::Create($target)
            try { $in.CopyTo($out) } finally { $out.Dispose(); $in.Dispose() }
            if ($f.sha256) {
                $s = [System.IO.File]::OpenRead($target)
                try { $h = Get-Sha256Hex $s } finally { $s.Dispose() }
                if ($h -ne ([string]$f.sha256).ToLowerInvariant()) { throw "$rel changed while it was staged" }
            }
        }
        # Swap: old files aside, new files in; the launcher last.
        $order = @($union | Where-Object { $_ -ne 'bdf.exe' -and $_ -ne $PackageFile }) + @($PackageFile) + @($union | Where-Object { $_ -eq 'bdf.exe' })
        foreach ($rel in $order) {
            $live = Join-Path $app ($rel -replace '/', '\')
            $aside = Join-Path $prev ($rel -replace '/', '\')
            if (Test-Path -LiteralPath $live -PathType Leaf) {
                New-Item -ItemType Directory -Path (Split-Path $aside -Parent) -Force | Out-Null
                Move-Item -LiteralPath $live -Destination $aside
                [void]$moved.Add(@($live, $aside))
            }
            $new = Join-Path $stage ($rel -replace '/', '\')
            if (Test-Path -LiteralPath $new -PathType Leaf) {
                New-Item -ItemType Directory -Path (Split-Path $live -Parent) -Force | Out-Null
                Move-Item -LiteralPath $new -Destination $live
                [void]$placed.Add($live)
            }
        }
        # Verify the installation against the list.
        foreach ($f in $listed) {
            $live = Join-Path $app (([string]$f.path) -replace '/', '\')
            $s = [System.IO.File]::OpenRead($live)
            try { $h = Get-Sha256Hex $s } finally { $s.Dispose() }
            if ($h -ne ([string]$f.sha256).ToLowerInvariant()) { throw "$($f.path) does not match the release after copying" }
        }
    } catch {
        $reason = $_.Exception.Message
        foreach ($p in $placed) { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue }
        for ($i = $moved.Count - 1; $i -ge 0; $i--) { Move-Item -LiteralPath $moved[$i][1] -Destination $moved[$i][0] -Force -ErrorAction SilentlyContinue }
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $prev -Recurse -Force -ErrorAction SilentlyContinue
        throw "rolled back: $reason"
    } finally { $zip.Dispose() }

    # Keep the displaced version as BDFPrevious (what the in-app rollback uses), like the in-app updater.
    $keep = Join-Path $app 'BDFPrevious'
    if ($moved.Count -gt 0) {
        if (Test-Path -LiteralPath $keep) { Remove-Item -LiteralPath $keep -Recurse -Force }
        Move-Item -LiteralPath $prev -Destination $keep
    } else { Remove-Item -LiteralPath $prev -Recurse -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue

    if ($profile.Sid) {
        # The person owns their copy and can modify it (so BDF updates itself without administrator rights).
        # Nothing else about the profile's permissions is touched.
        $sidArg = '*' + $profile.Sid
        & icacls.exe $app /setowner $sidArg /T /C /Q | Out-Null
        & icacls.exe $app /grant "${sidArg}:(OI)(CI)M" /T /C /Q | Out-Null
    }
    if ($installedVersion) { "updated $installedVersion -> $version" } else { "installed $version" }
}

$profiles = Get-TargetProfiles
if (@($profiles).Count -eq 0) { Write-Log 'no user profiles to deploy to' }
foreach ($p in $profiles) {
    try {
        $outcome = Install-ForProfile $p
        switch -Wildcard ($outcome) {
            'current' { Write-Log "$($p.Name): already has $version or newer; nothing to do"; [void]$Done.Add($p.Name) }
            'in-use' { Write-Log "$($p.Name): BDF is running from its folder; skipped (the next run will update it)"; [void]$Skipped.Add($p.Name) }
            default { Write-Log "$($p.Name): $outcome"; [void]$Done.Add($p.Name) }
        }
    } catch {
        Write-Log "$($p.Name): FAILED: $($_.Exception.Message)"
        [void]$Failed.Add($p.Name)
    }
}
Write-Log ("summary: done {0}, skipped (in use) {1}, failed {2}" -f $Done.Count, $Skipped.Count, $Failed.Count)
if ($Failed.Count -gt 0) { exit 1 }
exit 0
