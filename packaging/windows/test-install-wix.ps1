<#
.SYNOPSIS
  Test that install-wix.ps1 installs only the package whose hash it verified.

.DESCRIPTION
  Runs install-wix.ps1 for real (it needs dotnet and the network for the one official download):
    1. refusals: a tampered package and a wrong recorded hash are refused and leave nothing installed;
    2. the normal install succeeds, the package NuGet recorded is the verified one, and `wix --version` runs;
    3. isolation: an ambient NuGet source (a nuget.config in the working directory) offers a different
       `wix` 5.0.2. A control shows that a default `dotnet tool install` (the shape the script used before,
       with --add-source) does consult that source, so the test is not vacuous; the script's install,
       run from the same directory, still gets exactly the verified package and none of the poison.
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

try {
  # 2 first, because it also fetches the official package for the other tests to copy.
  $err = Run-Install 'positive'
  Check ($null -eq $err) "the normal install succeeds ($err)"
  $official = Join-Path $root "positive\feed\wix.$version.nupkg"
  Check ((Test-Path $official) -and (Sha512-Of $official) -ceq $expected) 'the package in the feed is the verified one'
  $records = @(Get-ChildItem (Join-Path $root 'positive') -Recurse -Filter "wix.$version.nupkg.sha512" -File)
  Check ($records.Count -ge 1 -and (($records | ForEach-Object { (Get-Content $_.FullName -Raw).Trim() }) | Where-Object { $_ -cne $expected }).Count -eq 0) 'NuGet recorded the verified hash for what it installed'
  $wix = Join-Path $root 'positive\tools\wix.exe'
  Check ((Test-Path $wix) -and ((& $wix --version) -match "^$([regex]::Escape($version))")) 'the installed wix runs and reports its version'

  # 1. refusals
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
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::Open($poison, 'Update')
  try {
    @($zip.Entries | Where-Object { $_.FullName -like '*.signature.p7s' }) | ForEach-Object { $_.Delete() }
    $entry = $zip.CreateEntry('poison.txt'); $w = New-Object IO.StreamWriter($entry.Open()); $w.Write('not the verified package'); $w.Dispose()
  } finally { $zip.Dispose() }
  Check ((Sha512-Of $poison) -cne $expected) 'the poisoned package differs from the verified one'
  $cwd = Join-Path $root 'cwd'
  New-Item -ItemType Directory -Force -Path $cwd | Out-Null
  $ambient = '<?xml version="1.0" encoding="utf-8"?><configuration><packageSources><add key="poison" value="' + [Security.SecurityElement]::Escape($poisonDir) + '" /></packageSources></configuration>'
  [IO.File]::WriteAllText((Join-Path $cwd 'nuget.config'), $ambient, (New-Object Text.UTF8Encoding($false)))

  Push-Location $cwd
  try {
    # Control: the earlier shape of the install (verified folder added to the ambient sources).
    $legacy = (dotnet tool install wix --version $version --tool-path (Join-Path $root 'legacy') --add-source (Join-Path $root 'positive\feed') --verbosity detailed 2>&1 | Out-String)
    Check ($legacy.Contains($poisonDir)) 'control: a default install consults the ambient poisoned source (so the isolation test means something)'
    # The script, from the same directory with the same ambient source.
    $err = Run-Install 'isolated' @{ Nupkg = $official }
    Check ($null -eq $err) "the install still succeeds next to a poisoned ambient source ($err)"
  } finally { Pop-Location }
  $leak = @(Get-ChildItem (Join-Path $root 'isolated') -Recurse -Filter 'poison.txt' -File -ErrorAction SilentlyContinue)
  Check ($leak.Count -eq 0) 'no byte of the poisoned package reached the isolated install'
  $rec = @(Get-ChildItem (Join-Path $root 'isolated') -Recurse -Filter "wix.$version.nupkg.sha512" -File)
  Check ($rec.Count -ge 1 -and (($rec | ForEach-Object { (Get-Content $_.FullName -Raw).Trim() }) | Where-Object { $_ -cne $expected }).Count -eq 0) 'the isolated install recorded the verified hash'
} finally {
  Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failures.Count) { throw "$($failures.Count) check(s) failed: $($failures -join '; ')" }
Write-Output 'all install-wix checks passed'
