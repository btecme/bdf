<#
.SYNOPSIS
  Test that install-wix.ps1 installs only the package whose hash it verified.

.DESCRIPTION
  Runs install-wix.ps1 for real (it needs dotnet and the network for the one official download):
    1. the normal install succeeds, and the installed wix.dll is byte-for-byte the verified package's;
    2. refusals: a tampered package and a wrong recorded hash are refused and install nothing;
    3. isolation: an ambient NuGet source (a nuget.config in the working directory, which here is the only
       source) offers a different `wix` 5.0.2. A control shows that a default `dotnet tool install` takes
       the package from that source, so the test is not vacuous; the script's install, run from the same
       directory, still gets exactly the verified package and none of the poison.
  It changes nothing outside temporary folders. `wix --version` is run from the verified install.

.EXAMPLE
  pwsh packaging/windows/test-install-wix.ps1
#>
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'install-wix.ps1'
$text = Get-Content -LiteralPath $script -Raw
if ($text -notmatch "\[string\] \`$Sha512 = '([^']+)'") { throw 'could not read the recorded SHA-512 from install-wix.ps1' }
$expected = $Matches[1]
$version = '5.0.2'
$root = Join-Path ([IO.Path]::GetTempPath()) ('wix-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $root | Out-Null
$failures = New-Object System.Collections.Generic.List[string]
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Check([bool] $ok, [string] $what) {
  if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $failures.Add($what) }
}

function Run-Install([string] $Name, [hashtable] $Extra = @{}) {
  # Returns $null on success, or the error text; the script throws on every refusal.
  # A hashtable splat binds the names; an array splat would pass them as plain strings.
  try { & $script -Work (Join-Path $root $Name) @Extra | Out-Null; return $null } catch { return $_.Exception.Message }
}

function Sha512-Of([string] $Path) {
  $sha = [System.Security.Cryptography.SHA512]::Create()
  $s = [IO.File]::OpenRead($Path)
  try { [Convert]::ToBase64String($sha.ComputeHash($s)) } finally { $s.Dispose() }
}

# SHA-256 of tools/net6.0/any/wix.dll inside the verified package, and of the copy installed under a work folder.
function Package-DllHash([string] $Package) {
  $zip = [IO.Compression.ZipFile]::OpenRead($Package)
  try {
    $entry = $zip.Entries | Where-Object { $_.FullName -eq 'tools/net6.0/any/wix.dll' } | Select-Object -First 1
    if (-not $entry) { return $null }
    $sha = [System.Security.Cryptography.SHA256]::Create(); $s = $entry.Open()
    try { return [BitConverter]::ToString($sha.ComputeHash($s)).Replace('-', '') } finally { $s.Dispose() }
  } finally { $zip.Dispose() }
}
function Installed-DllHash([string] $Name) {
  $dll = Join-Path $root "$Name\tools\.store\wix\$version\wix\$version\tools\net6.0\any\wix.dll"
  if (-not (Test-Path -LiteralPath $dll)) { return $null }
  return (Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash
}

try {
  # 1. the normal install (it also fetches the official package for the other tests to copy)
  $err = Run-Install 'positive'
  Check ($null -eq $err) "the normal install succeeds ($err)"
  $official = Join-Path $root "positive\feed\wix.$version.nupkg"
  Check ((Test-Path $official) -and (Sha512-Of $official) -ceq $expected) 'the package in the feed is the verified one'
  $want = Package-DllHash $official
  Check ($want -and ((Installed-DllHash 'positive') -eq $want)) 'the installed wix.dll is byte-for-byte the verified package''s'
  $wix = Join-Path $root 'positive\tools\wix.exe'
  Check ((Test-Path $wix) -and ((& $wix --version) -match "^$([regex]::Escape($version))")) 'the installed wix runs and reports its version'

  # 2. refusals
  $tampered = Join-Path $root 'tampered.nupkg'
  Copy-Item $official $tampered
  $bytes = [IO.File]::ReadAllBytes($tampered); $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 1; [IO.File]::WriteAllBytes($tampered, $bytes)
  $err = Run-Install 'tampered' @{ Nupkg = $tampered }
  Check ($err -and $err -match 'not installing it') 'a tampered package is refused'
  Check (-not (Test-Path (Join-Path $root 'tampered\tools'))) 'nothing was installed from the tampered package'
  $err = Run-Install 'wronghash' @{ Nupkg = $official; Sha512 = ('A' * 86 + '==') }
  Check ($err -and $err -match 'not installing it') 'a wrong recorded hash is refused'
  Check (-not (Test-Path (Join-Path $root 'wronghash\tools'))) 'nothing was installed under a wrong recorded hash'

  # 3. isolation from an ambient source
  $poisonDir = Join-Path $root 'poison'
  New-Item -ItemType Directory -Force -Path $poisonDir | Out-Null
  $poison = Join-Path $poisonDir "wix.$version.nupkg"
  Copy-Item $official $poison
  $zip = [IO.Compression.ZipFile]::Open($poison, 'Update')
  try {
    @($zip.Entries | Where-Object { $_.FullName -like '*.signature.p7s' }) | ForEach-Object { $_.Delete() }
    # A marker file, and an executable that differs from the real one by one appended byte.
    $entry = $zip.CreateEntry('poison.txt'); $w = New-Object IO.StreamWriter($entry.Open()); $w.Write('not the verified package'); $w.Dispose()
    $dllEntry = $zip.GetEntry('tools/net6.0/any/wix.dll')
    $ms = New-Object IO.MemoryStream; $in = $dllEntry.Open(); $in.CopyTo($ms); $in.Dispose(); $ms.WriteByte(0)
    $dllEntry.Delete()
    $replacement = $zip.CreateEntry('tools/net6.0/any/wix.dll'); $out = $replacement.Open(); $out.Write($ms.ToArray(), 0, [int]$ms.Length); $out.Dispose()
  } finally { $zip.Dispose() }
  Check ((Sha512-Of $poison) -cne $expected) 'the poisoned package differs from the verified one'
  $cwd = Join-Path $root 'cwd'
  New-Item -ItemType Directory -Force -Path $cwd | Out-Null
  $ambient = '<?xml version="1.0" encoding="utf-8"?><configuration><packageSources><clear /><add key="poison" value="' + [Security.SecurityElement]::Escape($poisonDir) + '" /></packageSources></configuration>'
  [IO.File]::WriteAllText((Join-Path $cwd 'nuget.config'), $ambient, (New-Object Text.UTF8Encoding($false)))

  Push-Location $cwd
  try {
    # Control: a default install honours the ambient configuration, so an ambient source CAN supply the tool
    # (the poisoned folder is the only ambient source here, which makes the outcome deterministic).
    $legacyOut = (dotnet tool install wix --version $version --tool-path (Join-Path $root 'legacy') 2>&1 | Out-String)
    $reached = @(Get-ChildItem (Join-Path $root 'legacy') -Recurse -Filter 'poison.txt' -File -ErrorAction SilentlyContinue)
    Check ($reached.Count -ge 1) "control: a default install takes the package from the ambient source, so the isolation test means something ($($legacyOut.Trim() -replace '\s+', ' '))"
    # The script, from the same directory with the same ambient source.
    $err = Run-Install 'isolated' @{ Nupkg = $official }
    Check ($null -eq $err) "the install still succeeds next to a poisoned ambient source ($err)"
  } finally { Pop-Location }
  $leak = @(Get-ChildItem (Join-Path $root 'isolated') -Recurse -Filter 'poison.txt' -File -ErrorAction SilentlyContinue)
  Check ($leak.Count -eq 0) 'no byte of the poisoned package reached the isolated install'
  Check ($want -and ((Installed-DllHash 'isolated') -eq $want)) 'the isolated install holds the verified wix.dll'
} finally {
  Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures.Count) { throw "$($failures.Count) check(s) failed: $($failures -join '; ')" }
Write-Output 'all install-wix checks passed'
