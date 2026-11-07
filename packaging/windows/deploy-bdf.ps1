<#
.SYNOPSIS
  Deploys the BDF portable package into each user's own profile (Action1 / any Local System deployment tool).

.DESCRIPTION
  Runs as Local System. It never starts BDF, never runs anything from the package or from a user's folder as
  Local System, and never touches a user's BDFData folder. The person's folder is theirs and they can change it at any
  moment, so Local System does not change anything inside it. What it does:

    1. Verifies the release exactly as the in-app updater does: the release file (bdf-update.json) is an
       envelope whose signature (ECDSA P-256 over SHA-256) is checked against -PublicKey, then the package's
       size and SHA-256 against that signed information, then every file in the ZIP against the package's own
       list. Nothing is read from an unauthenticated source. Unsigned or tampered input stops the run.
    2. Puts the verified release in a working folder only Local System and Administrators can change.
    3. For each intended user profile, starts a short task that runs AS THAT PERSON (a scheduled task with that
       person's own identity and no password: the Task Scheduler "S4U" logon, limited rights, no network). That
       task verifies the release again and installs (or updates) the portable copy in
       <profile>\AppData\Local\Programs\BDF by renaming files, with the previous version kept in BDFPrevious. Because
       it runs with the person's own rights it can reach nothing the person cannot already change, whatever they
       do to links and folders while it runs, and the files it writes are the person's own (no ownership or
       permission change is needed). A copy that is running, or any other open file, is never replaced: that
       user is skipped and picked up by the next run. If the task cannot be started as that person, that person
       is reported as failed and nothing is changed: there is no fallback that works in their folder as Local System.
    4. An interrupted run (the script or the machine stopped part-way) leaves a marker in the person's BDFUpdate
       folder; the next run finishes the job from the verified package instead of treating the copy as current.

  New users: -IncludeDefaultProfile also installs into C:\Users\Default, which Windows copies into every
  profile created later (only administrators can write there, so it is done directly). Users who already exist
  are covered by running this script again (Action1 policy).

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
.PARAMETER SkipLoggedOnUsers  Skip a person whose profile is loaded (they appear to be signed in), so their BDF is not updated under them.
                        This is about not disturbing a person at work. It is NOT a security control and it does not remove any
                        file-system race: the check is made once, a person can sign in a moment later, and a profile can be
                        loaded by things other than a sign-in. Safety comes from the install running as the person (above).
.PARAMETER AsUser       Internal. The folder of one profile to install into, used by the task Local System starts as that person.
                        Never needed by hand; run by hand it installs into the folder with the rights of whoever runs it.
.PARAMETER OutcomeFile  Internal. Where an -AsUser run writes a one-line result for the Local System run to read (as text, never as an authority).
.PARAMETER WhatIf       Verify and report what would happen without changing anything.

.NOTES
  The boundary. A script running as Local System must not work inside a folder a person can change: any check of a path
  ("is this a link?") is out of date by the time the path is used, and the person can swap a folder for a junction in
  between (measured: packaging\windows\test-deploy-race.ps1 steers the old in-place version through the swapped link). So:
    * Local System verifies the release, copies it to a folder only it and Administrators can write, and starts the install
      as the person (S4U scheduled task, limited rights, no network). The install can therefore only reach what that person can
      already change. Local System itself does not read, list, create, move or delete anything inside a person's profile,
      except that it reads the profile list from the registry and, for the Default profile (administrators only), installs directly.
    * Inside the person's own task the older checks remain, because they protect the person from a damaged copy, not Local
      System from the person: the installed package list (bdf-package.json) belongs to the person, so it is checked like the
      new list before anything moves (version, schema, count, every path: no BDFData, BDFUpdate or BDFPrevious, no
      absolute, drive, stream, short (8.3) or back-slash names, no traversal, no duplicates); the BDF folder is scanned for
      links before any change and each path is checked immediately before it is moved, created or deleted; deleting never
      follows a link; BDFData is never read, listed or changed.
    * Nothing from a profile is run as Local System, the working directory is System32, and the log and working folders
      are used only when they are not links and are owned by SYSTEM or Administrators (a new one is readable by them only).
    * Limits, stated plainly: the person's task runs with the person's rights, so a person who wants to damage their own BDF
      copy can. S4U needs the person's account to be allowed to run a batch task; where it is not (policy, or an account
      type Task Scheduler cannot run without a password) that person is reported as failed, and the in-app updater and
      the per-user ZIP still work for them. The window between "the script finished its checks" and "the file was moved"
      is not closed by checking harder; it is made harmless by who does the moving.

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
    [string]$AsUser,
    [string]$OutcomeFile,
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
if (-not $AsUser) { try { $LogDir = Initialize-LogDirectory } catch { $LogDir = $null } }

function Write-Log([string]$Message) {
    $line = '{0:u} {1}' -f (Get-Date), $Message
    Write-Output $line
    if ($LogDir) { try { Add-Content -LiteralPath (Join-Path $LogDir 'deploy.log') -Value $line -Encoding UTF8 } catch { } }
    # In the person's own task the only channel back is this file (text for the log, never an authority).
    if ($AsUser -and $OutcomeFile) { try { Add-Content -LiteralPath $OutcomeFile -Value $line -Encoding UTF8 } catch { } }
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
    if (-not (Test-NoReparse $profile.Path $app)) { throw 'a link or junction is in the way of the BDF folder; refusing to write through it' }
    $installedVersion = $null
    $oldFiles = @()
    $pendingMarker = Join-Path $app 'BDFUpdate\deploy-pending.json'
    $interrupted = $false
    $createdApp = -not (Test-Path -LiteralPath $app)   # a first install: if it fails, the folder it made goes again
    if (-not $createdApp) {
        # Nothing below is trusted until it has been checked: the folder is the person's, and so is the list in it.
        $oldListPath = Join-Path $app $PackageFile
        $hasList = (Test-Path -LiteralPath $oldListPath -PathType Leaf) -and -not (Test-IsLink $oldListPath)
        $hasMarker = Test-Path -LiteralPath $pendingMarker -PathType Leaf
        # No list is a managed copy only when it is a first install that did not finish: our marker is there, or the folder holds nothing but our own folders (the list is placed last, so it never was).
        $onlyOurs = @(Get-ChildItem -LiteralPath $app -Force | Where-Object { $_.Name -ine 'BDFUpdate' -and $_.Name -ine 'BDFPrevious' -and $_.Name -ine 'BDFData' }).Count -eq 0
        if (-not $hasList -and -not $hasMarker -and -not $onlyOurs) { throw "a folder exists at $app that is not a managed BDF copy; leaving it alone" }
        $link = Find-Link $app
        if ($link) { throw "a link or junction is inside the BDF folder ($link); refusing to work in it" }
        if ($hasList) {
            try {
                if ((Get-Item -LiteralPath $oldListPath -Force).Length -gt 1048576) { throw 'it is too large' }
                $oldInfo = Get-Content -LiteralPath $oldListPath -Raw -Encoding UTF8 | ConvertFrom-Json
                $oldFiles = @(Get-ValidatedPaths $oldInfo 'the installed package list')
                $installedVersion = [version][string]$oldInfo.version
            } catch { throw "the installed copy's package list cannot be trusted ($($_.Exception.Message)); leaving it exactly as it is" }
        }
        # A marker left by a run that did not finish means the copy may be part old, part new whatever its list says.
        $interrupted = $hasMarker -or -not $hasList
        if ($hasList -and $installedVersion -ge $version -and -not $interrupted) { return 'current' }
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
    # The staging folder is always rebuilt. The folder of displaced files is kept when an earlier run was interrupted: it holds the
    # version the person had, which must not be thrown away by finishing the job.
    Assert-Inside $app $stage -Internal; Remove-Tree $stage
    if (-not $interrupted) { Assert-Inside $app $prev -Internal; Remove-Tree $prev }
    New-SafeDirectory $app $stage
    New-SafeDirectory $app $prev

    # What every path must hash to once the update is in place (the package list itself is hashed as it is staged).
    $expected = @{}
    foreach ($f in $listed) { $expected[([string]$f.path).ToLowerInvariant()] = ([string]$f.sha256).ToLowerInvariant() }

    # The paths the version the person had owned. After an interruption the live list may already be the new one, but the old list
    # was set aside when it was replaced.
    $oldSet = @{}
    foreach ($r in $oldFiles) { $oldSet[([string]$r).ToLowerInvariant()] = $true }
    $asideList = Join-Path $prev $PackageFile
    if ($interrupted -and (Test-Path -LiteralPath $asideList -PathType Leaf)) {
        try {
            $ai = Get-Content -LiteralPath $asideList -Raw -Encoding UTF8 | ConvertFrom-Json
            $oldSet = @{}
            foreach ($r in @(Get-ValidatedPaths $ai 'the displaced package list')) { $oldSet[([string]$r).ToLowerInvariant()] = $true }
        } catch { }
    }
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
            $s = [System.IO.File]::OpenRead($target)
            try { $h = Get-Sha256Hex $s } finally { $s.Dispose() }
            if ($f.sha256) {
                if ($h -ne ([string]$f.sha256).ToLowerInvariant()) { throw "$rel changed while it was staged" }
            } else { $expected[$PackageFile] = $h }
        }
        # From here the copy may be part old, part new. The marker tells the next run so, if this one does not finish.
        [System.IO.File]::WriteAllText($pendingMarker, ('{{"version":"{0}","started":"{1:o}"}}' -f $version, (Get-Date)))
        # Swap: old files aside, new files in; the launcher last. Resuming after an interruption, a path that already holds
        # the new file is left alone and one whose old file is already aside is not displaced twice.
        $order = @($union | Where-Object { $_ -ne 'bdf.exe' -and $_ -ne $PackageFile }) + @($PackageFile) + @($union | Where-Object { $_ -eq 'bdf.exe' })
        foreach ($rel in $order) {
            $live = Join-Path $app ($rel -replace '/', '\')
            $aside = Join-Path $prev ($rel -replace '/', '\')
            $new = Join-Path $stage ($rel -replace '/', '\')
            Assert-Inside $app $live
            Assert-Inside $app $aside -Internal
            Assert-Inside $app $new -Internal
            $inPlace = $false
            if (Test-Path -LiteralPath $live -PathType Leaf) {
                if ($interrupted -and $expected.ContainsKey($rel.ToLowerInvariant())) {
                    $s = [System.IO.File]::OpenRead($live)
                    try { $inPlace = (Get-Sha256Hex $s) -eq $expected[$rel.ToLowerInvariant()] } finally { $s.Dispose() }
                }
                if (-not $inPlace) {
                    if (Test-Path -LiteralPath $aside) { throw "$rel has a file in the way and its earlier version is already aside; leaving it for the person to look at" }
                    New-SafeDirectory $app (Split-Path $aside -Parent)
                    Move-Item -LiteralPath $live -Destination $aside
                    [void]$moved.Add(@($live, $aside))
                } elseif (-not (Test-Path -LiteralPath $aside) -and $oldSet.ContainsKey($rel.ToLowerInvariant())) {
                    # Identical in both versions and never displaced (an earlier run stopped first): the previous version still needs it.
                    New-SafeDirectory $app (Split-Path $aside -Parent)
                    Copy-Item -LiteralPath $live -Destination $aside
                }
            }
            if (-not $inPlace -and (Test-Path -LiteralPath $new -PathType Leaf)) {
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
        if ($interrupted) {
            # An earlier run's moves are not in this run's lists, so rolling back here would be half a rollback. The marker stays,
            # the displaced files stay where they are, and the next run tries again from the verified package.
            throw "not finished (an earlier run was also interrupted): $reason"
        }
        foreach ($p in $placed) { try { Remove-Tree $p } catch { } }
        for ($i = $moved.Count - 1; $i -ge 0; $i--) { Move-Item -LiteralPath $moved[$i][1] -Destination $moved[$i][0] -Force -ErrorAction SilentlyContinue }
        try { Remove-Tree $stage } catch { }
        try { Remove-Tree $prev } catch { }
        try { Remove-Tree $pendingMarker } catch { }
        if ($createdApp) {
            # A first install that failed leaves nothing behind that a later run would refuse as "not a managed copy"; the person's data, if any, stays.
            try {
                foreach ($c in @(Get-ChildItem -LiteralPath $app -Force | Where-Object { $_.Name -ine 'BDFData' })) { Remove-Tree $c.FullName }
                if (@(Get-ChildItem -LiteralPath $app -Force).Count -eq 0) { [System.IO.Directory]::Delete($app) }
            } catch { }
        }
        throw "rolled back: $reason"
    } finally { $zip.Dispose() }

    # Keep the displaced version as BDFPrevious (what the in-app rollback uses), like the in-app updater.
    $keep = Join-Path $app 'BDFPrevious'
    Assert-Inside $app $keep -Internal
    $displaced = @(Get-ChildItem -LiteralPath $prev -Recurse -File -Force | Select-Object -First 1).Count -gt 0
    if ($displaced) {
        Remove-Tree $keep
        Move-Item -LiteralPath $prev -Destination $keep
    } else { Remove-Tree $prev }
    Remove-Tree $stage
    Remove-Tree $pendingMarker
    if ($installedVersion) { "updated $installedVersion -> $version" } else { "installed $version" }
}

# ---- 3. as the person: the install itself ---------------------------------------------------------------------------

# This is what the task started as the person runs (and what a direct run does for the Default profile and for -UsersRoot).
if ($AsUser) {
    $me = [pscustomobject]@{ Name = (Split-Path $AsUser -Leaf); Path = $AsUser; Sid = $null }
    try {
        $o = Install-ForProfile $me
        Write-Log "RESULT: $o"
        if ($o -eq 'in-use') { exit 4 }
        exit 0
    } catch {
        Write-Log "RESULT: FAILED: $($_.Exception.Message)"
        exit 1
    }
}

# ---- 4. as Local System: hand each person's install to that person ------------------------------------------------------

# A folder only Local System and Administrators can change, holding the verified release the people's tasks read.
function New-RunFolder {
    if (-not $LogDir) { throw 'there is no safe working folder (the log folder is missing, is a link, or is not owned by SYSTEM or Administrators)' }
    $run = Join-Path $LogDir ('run-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $run | Out-Null
    & (Join-Path $System32 'icacls.exe') $run /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' /Q | Out-Null
    if ($LASTEXITCODE) { throw 'could not restrict the working folder' }
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $run 'deploy-bdf.ps1')
    Copy-Item -LiteralPath $zipInfo.FullName -Destination (Join-Path $run 'package.zip')
    Copy-Item -LiteralPath $ReleaseFile -Destination (Join-Path $run 'release.json')
    $run
}

# Text a person's task wrote, for the log only: a regular file, small, printable, a few lines.
function Read-Outcome([string]$Path) {
    try {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
        if (($item.Attributes -band $ReparseAttr) -or $item.PSIsContainer -or $item.Length -gt 16384) { return @() }
        $lines = @(Get-Content -LiteralPath $Path -Encoding UTF8 -Tail 4)
        @($lines | ForEach-Object { (($_ -replace '[^\x20-\x7e]', '?')) } | ForEach-Object { if ($_.Length -gt 300) { $_.Substring(0, 300) } else { $_ } })
    } catch { @() }
}

function Invoke-InstallAsUser($profile, [string]$Run) {
    $account = ([System.Security.Principal.SecurityIdentifier]$profile.Sid).Translate([System.Security.Principal.NTAccount]).Value
    $sidDir = Join-Path $Run $profile.Sid
    New-Item -ItemType Directory -Path $sidDir | Out-Null
    & (Join-Path $System32 'icacls.exe') $sidDir /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' "*$($profile.Sid):(OI)(CI)M" /Q | Out-Null
    if ($LASTEXITCODE) { throw 'could not prepare the working folder for this person' }
    $outcomeFile = Join-Path $sidDir 'outcome.txt'
    $arguments = @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $Run 'deploy-bdf.ps1')`"",
        '-PackageZip', "`"$(Join-Path $Run 'package.zip')`"", '-ReleaseFile', "`"$(Join-Path $Run 'release.json')`"",
        '-PublicKey', $PublicKey, '-Channel', $Channel, '-AsUser', "`"$($profile.Path)`"", '-OutcomeFile', "`"$outcomeFile`""
    )
    if ($WhatIf) { $arguments += '-WhatIf' }
    $taskName = 'BDF-deploy-' + [guid]::NewGuid().ToString('N').Substring(0, 12)
    $taskPath = '\BDF\'
    $action = New-ScheduledTaskAction -Execute (Join-Path $System32 'WindowsPowerShell\v1.0\powershell.exe') -Argument ($arguments -join ' ') -WorkingDirectory $System32
    $principal = New-ScheduledTaskPrincipal -UserId $account -LogonType S4U -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 30) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    try {
        try {
            Register-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Action $action -Principal $principal -Settings $settings -Force | Out-Null
            Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
        } catch { throw "could not start the install as $account ($($_.Exception.Message)); nothing was changed" }
        $deadline = (Get-Date).AddMinutes(31)
        $info = $null
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $state = (Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath).State
            $info = Get-ScheduledTaskInfo -TaskName $taskName -TaskPath $taskPath
            if ($state -eq 'Ready' -and $info.LastRunTime.Year -gt 2000) { break }
            $info = $null
        }
        if (-not $info) {
            Stop-ScheduledTask -TaskName $taskName -TaskPath $taskPath -ErrorAction SilentlyContinue
            throw "the install as $account did not finish in 30 minutes and was stopped; the next run will finish it"
        }
        $code = [int64]$info.LastTaskResult
        $lines = @(Read-Outcome $outcomeFile)
        $result = @($lines | Where-Object { $_ -match 'RESULT: ' } | Select-Object -Last 1)
        $message = if ($result.Count -gt 0) { ($result[0] -replace '^.*RESULT: ', '') } else { ($lines -join ' | ') }
        if ($code -eq 4) { return 'in-use' }
        if ($code -ne 0) { throw "the install as $account failed (task result $code): $message" }
        if ($message) { $message } else { 'done' }
    } finally {
        try { Unregister-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Confirm:$false -ErrorAction SilentlyContinue } catch { }
    }
}

# A run that was ended part-way (the deployment tool stopped it, the machine restarted) can leave its working folder and its
# per-person tasks behind. Anything of ours older than an hour is stale: a normal run removes its own as it goes.
function Clear-StaleRuns {
    $cutoff = (Get-Date).AddHours(-1)
    if ($LogDir) {
        foreach ($d in @(Get-ChildItem -LiteralPath $LogDir -Directory -Filter 'run-*' -Force -ErrorAction SilentlyContinue)) {
            if ($d.LastWriteTime -lt $cutoff -and -not ($d.Attributes -band $ReparseAttr)) { try { Remove-Tree $d.FullName; Write-Log "removed a stale working folder from an earlier run: $($d.Name)" } catch { } }
        }
    }
    try {
        foreach ($t in @(Get-ScheduledTask -TaskPath '\BDF\' -ErrorAction SilentlyContinue | Where-Object { $_.TaskName -like 'BDF-deploy-*' -and $_.State -ne 'Running' })) {
            $made = $null
            try { $made = [datetime]$t.Date } catch { }
            if ($made -and $made -lt $cutoff) { Unregister-ScheduledTask -TaskName $t.TaskName -TaskPath '\BDF\' -Confirm:$false -ErrorAction SilentlyContinue; Write-Log "removed a stale task from an earlier run: $($t.TaskName)" }
        }
    } catch { }
}
if (-not $UsersRoot) { Clear-StaleRuns }

$profiles = Get-TargetProfiles
if (@($profiles).Count -eq 0) { Write-Log 'no user profiles to deploy to' }
$run = $null
try {
    foreach ($p in $profiles) {
        try {
            if ($SkipLoggedOnUsers -and $p.Sid -and (Test-Path -LiteralPath "Registry::HKEY_USERS\$($p.Sid)")) { $outcome = 'signed-in' }
            elseif ($p.Sid) {
                if (-not $run) { $run = New-RunFolder }
                $outcome = Invoke-InstallAsUser $p $run
            } else { $outcome = Install-ForProfile $p }
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
} finally {
    if ($run) { try { Remove-Tree $run } catch { } }
}
Write-Log ("summary: done {0}, skipped (in use) {1}, failed {2}" -f $Done.Count, $Skipped.Count, $Failed.Count)
if ($Failed.Count -gt 0) { exit 1 }
exit 0
