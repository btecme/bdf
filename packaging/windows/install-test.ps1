<#
.SYNOPSIS
  Install a PdfCraft MSI on this machine, check what was installed, run the CLI, and uninstall it.

.DESCRIPTION
  The static MSI checks (test-msi.ps1) read the database without installing. This installs it for real
  (per machine, silently), then checks that:
    * pdfcraft.exe and pdfcraft-cli.exe are in Program Files\PdfCraft and are built for -Arch
      (the machine field of the PE header, so an x64 build cannot ship labelled arm64);
    * the OCR models folder was installed with -ExpectOcrModels, and in the core package (the default) is absent;
    * the CLI runs and reports a version, when the build can run on this machine;
    * uninstalling removes the executables.
  It needs an elevated session and changes the machine; run it on a CI runner or a throwaway VM.

.EXAMPLE
  pwsh packaging/windows/install-test.ps1 dist/pdfcraft-0.7.1-windows-arm64.msi -Arch arm64
#>
param(
  [Parameter(Mandatory)] [string] $Msi,
  [Parameter(Mandatory)] [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch,
  [switch] $ExpectOcrModels
)
$ErrorActionPreference = 'Stop'

$Msi = (Resolve-Path -LiteralPath $Msi).Path
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
# x86 and x64 code runs on arm64 Windows 11 and x86 runs on x64, but an arm64 build runs only on arm64.
$CanRun = switch ($Arch) { 'arm64' { $HostArch -eq 'arm64' } 'x64' { $HostArch -in 'x64', 'arm64' } 'x86' { $HostArch -in 'x86', 'x64', 'arm64' } }
# Program Files for the OS's native bitness: the MSI installs into ProgramFiles6432Folder.
$Dir = Join-Path $env:ProgramFiles 'PdfCraft'
$Log = Join-Path ([IO.Path]::GetTempPath()) 'pdfcraft-install.log'

function Invoke-Msi([string[]] $Arguments) {
  $p = Start-Process msiexec.exe -ArgumentList $Arguments -Wait -PassThru
  if ($p.ExitCode -ne 0) {
    if (Test-Path $Log) { Get-Content $Log -Tail 80 }
    throw "msiexec $($Arguments -join ' ') exited with $($p.ExitCode)"
  }
}

function Get-PeMachine([string] $Path) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  [BitConverter]::ToUInt16($bytes, [BitConverter]::ToInt32($bytes, 0x3C) + 4)
}

Write-Output "installing $Msi (host $HostArch, expecting a $Arch build)"
Invoke-Msi @('/i', "`"$Msi`"", '/qn', '/l*v', "`"$Log`"")
try {
  foreach ($exe in 'pdfcraft.exe', 'pdfcraft-cli.exe') {
    $path = Join-Path $Dir $exe
    if (-not (Test-Path -LiteralPath $path)) { throw "$exe was not installed in $Dir" }
    $m = Get-PeMachine $path
    if ($m -ne $Machine) { throw ("{0} is built for machine 0x{1:X}, expected 0x{2:X} ({3})" -f $exe, $m, $Machine, $Arch) }
    Write-Output "ok $path is a $Arch build"
  }
  $hasModels = Test-Path -LiteralPath (Join-Path $Dir 'models')
  if ($ExpectOcrModels) {
    if (-not $hasModels) { throw "the OCR models folder was not installed in $Dir" }
    Write-Output 'ok models folder installed'
  } else {
    if ($hasModels) { throw "the core package installed an OCR models folder in $Dir" }
    Write-Output 'ok no OCR models folder (core package)'
  }
  if ($CanRun) {
    $out = & (Join-Path $Dir 'pdfcraft-cli.exe') --version 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { Write-Output $out; throw "pdfcraft-cli.exe --version exited with $LASTEXITCODE" }
    if (-not $out.Trim()) { throw 'pdfcraft-cli.exe --version printed nothing' }
    Write-Output "ok pdfcraft-cli.exe --version: $($out.Trim())"
  } else {
    Write-Output "skipped: a $Arch build does not run on this $HostArch machine"
  }
} finally {
  Invoke-Msi @('/x', "`"$Msi`"", '/qn')
}
if (Test-Path -LiteralPath (Join-Path $Dir 'pdfcraft.exe')) { throw 'uninstall left pdfcraft.exe behind' }
Write-Output 'ok uninstalled'
