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
.PARAMETER LogDirectory Where the log is written (default C:\ProgramData\BDF, used only when it is safe: see Notes).
.PARAMETER SkipLoggedOnUsers  Skip a person whose profile is loaded (they are signed in), so nobody can change their own folder while
                        it is being updated. Without it a signed-in person's copy is still protected by the checks below.
.PARAMETER WhatIf       Verify and report what would happen without changing anything.

.NOTES
  Safety rules for a Local System run in folders the person can write to:
    * The installed package list (bdf-package.json) belongs to the person, so it is never an authority. It is checked like
      the new list before anything moves: version, schema, count, and every path (no BDFData, BDFUpdate or BDFPrevious, no
      absolute, drive, stream, short (8.3) or back-slash names, no traversal, no duplicates). An installed copy whose list
      fails is refused and left exactly as it is.
    * Before any change the whole BDF folder is scanned for links (junctions, symbolic links); one anywhere, except inside
      the person's own BDFData (which is never read, listed or changed), refuses that profile. Each path is also checked to
      stay inside the folder, and free of links, immediately before it is moved, created or deleted.
    * Deleting never follows a link (a link is removed, never what it points to), and ownership is applied per item
      without following links, never across BDFData.
    * Nothing from a profile is run, the working directory is System32, and the log folder is used only when it is not a
      link and is owned by SYSTEM or Administrators (a new one is created readable by them only).
    * What remains is a signed-in person racing the script inside a window of milliseconds; -SkipLoggedOnUsers closes it
      for deployments that can wait until nobody is signed in.

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
    [string]$LogDirectory,
    [switch]$SkipLoggedOnUsers,
    [switch]$WhatIf
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# `-File` passes "a,b" as one string: accept a comma-separated list as well as an array.
if ($Users) { $Users = @($Users | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }

# Never run from, or relative to, a user's folder.
try { Set-Location -LiteralPath (Join-Path $env:SystemRoot 'System32'); [Environment]::CurrentDirectory = (Get-Location).Path } catch { }
$System32 = Join-Path $env:SystemRoot 'System32'
$ReparseAttr = [System.IO.FileAttributes]::ReparsePoint
$AppSub = 'AppData\Local\Programs\BDF'
$PackageFile = 'bdf-package.json'
$Reserved = @('bdfdata', 'bdfupdate', 'bdfprevious')
$Skipped = New-Object System.Collections.ArrayList
$Failed = New-Object System.Collections.ArrayList
$Done = New-Object System.Collections.ArrayList

# The log folder is ours alone: not a link, and owned by SYSTEM or Administrators. An unsafe one is not written to
# (the output still reaches the deployment tool); a new one is created readable by SYSTEM and Administrators only.
$LogDir = $null
function Initialize-LogDirectory {
    if ($LogDirectory) { New-Item -ItemType Directory -Path $LogDirectory -Force | Out-Null; return $LogDirectory }
    $dir = Join-Path $env:ProgramData 'BDF'
    if (Test-Path -LiteralPath $dir) {
        $item = Get-Item -LiteralPath $dir -Force
        $owner = (Get-Acl -LiteralPath $dir).Owner
        if (($item.Attributes -band $ReparseAttr) -or -not $item.PSIsContainer -or $owner -notmatch '^(NT AUTHORITY\\SYSTEM|BUILTIN\\Administrators|NT SERVICE\\TrustedInstaller)$') { return $null }
        return $dir
    }
    New-Item -ItemType Directory -Path $dir | Out-Null
    & (Join-Path $System32 'icacls.exe') $dir /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /Q | Out-Null
    $dir
}
try { $LogDir = Initialize-LogDirectory } catch { $LogDir = $null }

function Write-Log([string]$Message) {
    $line = '{0:u} {1}' -f (Get-Date), $Message
    Write-Output $line
    if ($LogDir) { try { Add-Content -LiteralPath (Join-Path $LogDir 'deploy.log') -Value $line -Encoding UTF8 } catch { } }
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
        # 8.3 short names (BDFDAT~1) are another spelling of a long name, so they could reach a protected folder.
        if ($p -match '~[0-9]') { return $false }
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
            if ((Get-Item -LiteralPath $p -Force).Attributes -band $ReparseAttr) { return $false }
        }
        $p = Split-Path $p -Parent
    }
    $true
}

function Test-IsLink([string]$Path) {
    try { $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop } catch { return $false }
    [bool]($item.Attributes -band $ReparseAttr)
}

# A path about to be changed must be inside the BDF folder, outside the person's data and free of links all the way
# down from the folder. -Internal allows our own working folders (BDFUpdate, BDFPrevious); BDFData is never allowed.
function Assert-Inside([string]$App, [string]$Path, [switch]$Internal) {
    $appFull = [System.IO.Path]::GetFullPath($App).TrimEnd('\')
    $full = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    if ($full -ine $appFull) {
        if (-not $full.StartsWith($appFull + '\', [System.StringComparison]::OrdinalIgnoreCase)) { throw "refusing a path outside the BDF folder: $Path" }
        $first = $full.Substring($appFull.Length + 1).Split('\')[0].ToLowerInvariant()
        if ($Reserved -contains $first -and (-not $Internal -or $first -eq 'bdfdata')) { throw "refusing a path inside a protected folder: $first" }
    }
    $p = $full
    while ($p.Length -ge $appFull.Length) {
        if (Test-IsLink $p) { throw "a link appeared at $p; refusing to change anything through it" }
        $parent = Split-Path $p -Parent
        if (-not $parent -or $parent -eq $p) { break }
        $p = $parent
    }
}

# The first link (junction, symbolic link) inside the BDF folder, other than inside the person's own BDFData, which is
# never read, listed or changed. Returns $null when there is none.
function Find-Link([string]$App) {
    $stack = New-Object System.Collections.Stack
    $stack.Push($App)
    $seen = 0
    while ($stack.Count -gt 0) {
        $dir = [string]$stack.Pop()
        foreach ($e in @(Get-ChildItem -LiteralPath $dir -Force)) {
            $seen++
            if ($seen -gt 50000) { return "(more than 50000 entries)" }
            if ($e.Attributes -band $ReparseAttr) { return $e.FullName }
            if ($e.PSIsContainer -and -not ($dir -eq $App -and $e.Name -ieq 'BDFData')) { $stack.Push($e.FullName) }
        }
    }
    $null
}

# Delete a file or folder tree without ever following a link: a link is removed, never what it points to.
function Remove-Tree([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path) -and -not (Test-IsLink $Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band $ReparseAttr) {
        if ($item.PSIsContainer) { [System.IO.Directory]::Delete($item.FullName) } else { [System.IO.File]::Delete($item.FullName) }
        return
    }
    if ($item.PSIsContainer) {
        foreach ($c in @(Get-ChildItem -LiteralPath $item.FullName -Force)) { Remove-Tree $c.FullName }
        $item.Attributes = [System.IO.FileAttributes]::Directory
        [System.IO.Directory]::Delete($item.FullName)
    } else {
        $item.Attributes = [System.IO.FileAttributes]::Normal
        [System.IO.File]::Delete($item.FullName)
    }
}

function Test-InUse([string]$File) {
    if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { return $false }
    try {
        $fs = [System.IO.File]::Open($File, 'Open', 'ReadWrite', 'None')
        $fs.Dispose()
        $false
    } catch [System.IO.IOException] { $true } catch { $false }
}

# A package list is an input like any other, whoever wrote it: this returns its paths or throws. The installed copy's list
# is written by the person, so it gets exactly the checks the signed package's list gets, before anything is moved.
function Get-ValidatedPaths($Info, [string]$What) {
    if ($null -eq $Info -or $Info.schema -ne 1 -or $Info.product -ne 'bdf') { throw "$What is not a BDF package list" }
    if ([string]$Info.version -notmatch '^\d+\.\d+\.\d+$') { throw "$What has no valid version" }
    $entries = @($Info.files)
    if ($entries.Count -eq 0 -or $entries.Count -gt 2000) { throw "$What lists an impossible number of files" }
    $seen = @{}
    $paths = New-Object System.Collections.ArrayList
    foreach ($f in $entries) {
        $p = [string]$f.path
        if (-not (Test-SafeRelativePath $p) -or $p -eq $PackageFile) { throw ("$What lists an unsafe path: " + $p.Substring(0, [Math]::Min(60, $p.Length))) }
        $k = $p.ToLowerInvariant()
        if ($seen.ContainsKey($k)) { throw "$What lists $p twice" }
        $seen[$k] = $true
        [void]$paths.Add($p)
    }
    $paths.ToArray()
}

function New-SafeDirectory([string]$App, [string]$Dir) {
    Assert-Inside $App $Dir -Internal
    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
}

function Install-ForProfile($profile) {
    $app = Join-Path $profile.Path $AppSub
    if ($SkipLoggedOnUsers -and $profile.Sid -and (Test-Path -LiteralPath "Registry::HKEY_USERS\$($profile.Sid)")) { return 'signed-in' }
    if (-not (Test-NoReparse $profile.Path $app)) { throw 'a link or junction is in the way of the BDF folder; refusing to write through it' }
    $installedVersion = $null
    $oldFiles = @()
    if (Test-Path -LiteralPath $app) {
        # Nothing below is trusted until it has been checked: the folder is the person's, and so is the list in it.
        $oldListPath = Join-Path $app $PackageFile
        if (-not (Test-Path -LiteralPath $oldListPath -PathType Leaf) -or (Test-IsLink $oldListPath)) { throw "a folder exists at $app that is not a managed BDF copy; leaving it alone" }
        $link = Find-Link $app
        if ($link) { throw "a link or junction is inside the BDF folder ($link); refusing to work in it" }
        try {
            if ((Get-Item -LiteralPath $oldListPath -Force).Length -gt 1048576) { throw 'it is too large' }
            $oldInfo = Get-Content -LiteralPath $oldListPath -Raw -Encoding UTF8 | ConvertFrom-Json
            $oldFiles = @(Get-ValidatedPaths $oldInfo 'the installed package list')
            $installedVersion = [version][string]$oldInfo.version
        } catch { throw "the installed copy's package list cannot be trusted ($($_.Exception.Message)); leaving it exactly as it is" }
        if ($installedVersion -ge $version) { return 'current' }
    }
    if ($WhatIf) { return $(if ($installedVersion) { "would update $installedVersion -> $version" } else { "would install $version" }) }

    # Every path to be changed: the new list plus the checked old one, each name once (the file system ignores case).
    $names = @{}
    $union = New-Object System.Collections.ArrayList
    foreach ($rel in @($oldFiles + @($listed | ForEach-Object { [string]$_.path }) + $PackageFile)) {
        if ($names.ContainsKey($rel.ToLowerInvariant())) { continue }
        $names[$rel.ToLowerInvariant()] = $true
        [void]$union.Add($rel)
    }
    # Files to be replaced must be free: a running BDF (or its CLI, worker or updater) is never replaced or ended.
    foreach ($rel in $union) {
        $live = Join-Path $app ($rel -replace '/', '\')
        Assert-Inside $app $live
        if (Test-InUse $live) { return 'in-use' }
    }
    New-Item -ItemType Directory -Path $app -Force | Out-Null
    $stage = Join-Path $app 'BDFUpdate\deploy-stage'
    $prev = Join-Path $app 'BDFUpdate\deploy-previous'
    foreach ($d in @($stage, $prev)) { Assert-Inside $app $d -Internal; Remove-Tree $d }
    New-SafeDirectory $app $stage
    New-SafeDirectory $app $prev

    $moved = New-Object System.Collections.ArrayList   # @(live, prev) pairs displaced, for rollback
    $placed = New-Object System.Collections.ArrayList  # live paths of new files put in place
    $zip = [System.IO.Compression.ZipFile]::OpenRead($zipInfo.FullName)
    try {
        # Stage every file, hashing as it is written.
        $all = @($listed) + [pscustomobject]@{ path = $PackageFile; size = $infoBytes.Length; sha256 = $null }
        foreach ($f in $all) {
            $rel = [string]$f.path
            $target = Join-Path $stage ($rel -replace '/', '\')
            Assert-Inside $app $target -Internal
            New-SafeDirectory $app (Split-Path $target -Parent)
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
            $new = Join-Path $stage ($rel -replace '/', '\')
            Assert-Inside $app $live
            Assert-Inside $app $aside -Internal
            Assert-Inside $app $new -Internal
            if (Test-Path -LiteralPath $live -PathType Leaf) {
                New-SafeDirectory $app (Split-Path $aside -Parent)
                Move-Item -LiteralPath $live -Destination $aside
                [void]$moved.Add(@($live, $aside))
            }
            if (Test-Path -LiteralPath $new -PathType Leaf) {
                New-SafeDirectory $app (Split-Path $live -Parent)
                Assert-Inside $app $live
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
        foreach ($p in $placed) { try { Remove-Tree $p } catch { } }
        for ($i = $moved.Count - 1; $i -ge 0; $i--) { Move-Item -LiteralPath $moved[$i][1] -Destination $moved[$i][0] -Force -ErrorAction SilentlyContinue }
        try { Remove-Tree $stage } catch { }
        try { Remove-Tree $prev } catch { }
        throw "rolled back: $reason"
    } finally { $zip.Dispose() }

    # Keep the displaced version as BDFPrevious (what the in-app rollback uses), like the in-app updater.
    $keep = Join-Path $app 'BDFPrevious'
    Assert-Inside $app $keep -Internal
    if ($moved.Count -gt 0) {
        Remove-Tree $keep
        Move-Item -LiteralPath $prev -Destination $keep
    } else { Remove-Tree $prev }
    Remove-Tree $stage

    if ($profile.Sid) {
        # The person owns their copy and can modify it (so BDF updates itself without administrator rights). Item by item,
        # without following links and never across BDFData; nothing else about the profile's permissions is touched.
        $link = Find-Link $app
        if ($link) { throw "BDF was installed, but a link appeared inside the folder ($link), so ownership was not changed" }
        $sidArg = '*' + $profile.Sid
        $icacls = Join-Path $System32 'icacls.exe'
        & $icacls $app /setowner $sidArg /C /Q /L | Out-Null
        & $icacls $app /grant "${sidArg}:(OI)(CI)M" /C /Q /L | Out-Null
        foreach ($c in @(Get-ChildItem -LiteralPath $app -Force)) {
            if ($c.Name -ieq 'BDFData') { continue }
            if ($c.Attributes -band $ReparseAttr) { throw "BDF was installed, but a link appeared inside the folder ($($c.FullName)), so ownership was not changed" }
            & $icacls $c.FullName /setowner $sidArg /T /C /Q /L | Out-Null
            & $icacls $c.FullName /grant "${sidArg}:(OI)(CI)M" /T /C /Q /L | Out-Null
        }
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
            'signed-in' { Write-Log "$($p.Name): signed in; skipped by -SkipLoggedOnUsers (the next run will update it)"; [void]$Skipped.Add($p.Name) }
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
