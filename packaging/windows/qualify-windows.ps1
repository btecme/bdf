<#
.SYNOPSIS
  Release qualification of the Windows portable updates and deployment on a DISPOSABLE Windows machine (BUILD-018).
.DESCRIPTION
  This creates local users and profiles, changes a local security policy, mounts a small virtual disk, registers scheduled tasks as
  Local System and starts and stops processes. It refuses to run anywhere that is not a machine thrown away afterwards: a GitHub-hosted
  runner, or a machine the owner has marked by creating the file C:\BDF-DISPOSABLE-MACHINE (a VM they will discard, Windows Sandbox).
  It never publishes or signs anything: it uses the throwaway key and packages made by qualify-build.ps1.

  Stages (each is independent and writes <EvidenceDir>\<stage>.txt):
    system    Deploy as Local System (as Action1 does) to real standard-user profiles: the install runs as each person (S4U), the files
              are theirs, a standard user can run and update their own copy, a second user cannot touch it, reruns do nothing, a running
              copy is skipped then updated, the Default profile reaches a user created later, an account that may not run batch tasks is
              reported as failed with nothing changed, -WhatIf changes nothing, and a run ended part-way is finished by the next run.
    boundary  The file-system race, with a real second process running as the person: previous script (measured, informational)
              against the current one (asserted: nothing outside the person's own rights is ever touched).
    disk      A really full volume: download, staging and an update on a 400 MB virtual disk, and the deployment script, at several
              amounts of free space; the copy must be exactly as it was and recover once there is room.
    desktop   The real window: Later, download, a cancelled close, the unsaved-work question, the update, the restart, the data; and
              the Windows integration (registration visible to Windows, then the default-app choice and a PDF double-click as far as
              they can be driven without a person).
.PARAMETER Stage                 system | boundary | disk | desktop
.PARAMETER Material              The folder qualify-build.ps1 wrote (bins, packages, feed, pubkey.txt).
.PARAMETER EvidenceDir           Where the result files and screenshots go.
.PARAMETER LegacyDeploy          The previous deploy-bdf.ps1 (boundary stage), to show the difference.
.PARAMETER ThisMachineIsDisposable  Required: you say this machine is thrown away afterwards.
#>
param(
    [Parameter(Mandatory)][ValidateSet('system', 'boundary', 'disk', 'desktop', 'probe')][string]$Stage,
    [string]$Material,
    [string]$EvidenceDir,
    [string]$LegacyDeploy,
    [int]$Port = 8089,
    [int]$Trials = 10,
    [switch]$ThisMachineIsDisposable
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if ($Stage -ne 'probe') { $Material = (Resolve-Path $Material).Path }
if (-not $EvidenceDir) { $EvidenceDir = Join-Path $Repo 'qualify-evidence' }
New-Item -ItemType Directory -Path $EvidenceDir -Force | Out-Null
$EvidenceDir = (Resolve-Path $EvidenceDir).Path
$Deploy = Join-Path $PSScriptRoot 'deploy-bdf.ps1'
if ($Stage -ne 'probe') {
    $Pub = (Get-Content (Join-Path $Material 'pubkey.txt') -Raw).Trim()
    $Versions = @(Get-Content (Join-Path $Material 'versions.txt'))
    $V0 = $Versions[0]; $V1 = $Versions[1]
}

# ---- the guard ------------------------------------------------------------------------------------------------------
$hosted = ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted')
$marked = Test-Path -LiteralPath 'C:\BDF-DISPOSABLE-MACHINE'
$sandbox = ($env:USERNAME -eq 'WDAGUtilityAccount')
if (-not ($ThisMachineIsDisposable -and ($hosted -or $marked -or $sandbox))) {
    Write-Output 'REFUSED: this script creates users, changes local policy, mounts disks and runs tasks as Local System.'
    Write-Output 'It runs only on a disposable machine: a GitHub-hosted runner, Windows Sandbox, or a VM you have marked by creating'
    Write-Output 'C:\BDF-DISPOSABLE-MACHINE, and only with -ThisMachineIsDisposable. This machine is none of those (or the switch is missing).'
    exit 3
}
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Output 'REFUSED: run this elevated (as an administrator) on the disposable machine.'
    exit 3
}

# ---- reporting ------------------------------------------------------------------------------------------------------
$Report = Join-Path $EvidenceDir ("$Stage-" + $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant() + '.txt')
Set-Content -Path $Report -Value ("BDF qualification, stage '$Stage', {0:u} on {1} ({2})" -f (Get-Date), $env:COMPUTERNAME, [Environment]::OSVersion.VersionString) -Encoding utf8
$script:failures = 0
function Say([string]$m) { Write-Output $m; Add-Content -Path $Report -Value $m -Encoding utf8 }
function Check([bool]$ok, [string]$what) { if ($ok) { Say "ok    $what" } else { Say "FAIL  $what"; $script:failures++ } }
function Info([string]$m) { Say "info  $m" }
function Scenario([string]$name, [scriptblock]$body) {
    Say ''
    Say "== $name"
    try { & $body } catch { Say "FAIL  $name stopped with an error: $($_.Exception.Message)"; $script:failures++ }
}

# ---- helpers: users, profiles, tasks ----------------------------------------------------------------------------------
Add-Type -Namespace BdfQ -Name Profile -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("userenv.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
public static extern int CreateProfile([System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.LPWStr)] string pszUserSid,
    [System.Runtime.InteropServices.MarshalAs(System.Runtime.InteropServices.UnmanagedType.LPWStr)] string pszUserName,
    [System.Runtime.InteropServices.Out] System.Text.StringBuilder pszProfilePath, uint cchProfilePath);
'@
Add-Type -Namespace BdfQ -Name Assoc -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shlwapi.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
public static extern int AssocQueryString(uint flags, int str, string pszAssoc, string pszExtra, [System.Runtime.InteropServices.Out] System.Text.StringBuilder pszOut, ref uint pcchOut);
'@
function Assoc-Command([string]$ext) { $sb = New-Object System.Text.StringBuilder 1024; [uint32]$n = 1024; $hr = [BdfQ.Assoc]::AssocQueryString(0, 1, $ext, 'open', $sb, [ref]$n); if ($hr -eq 0) { $sb.ToString() } else { "(none: 0x{0:X8})" -f $hr } }
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
function New-Password {
    $b = New-Object byte[] 18; $rng.GetBytes($b)
    'Aa1!' + ([Convert]::ToBase64String($b) -replace '[^A-Za-z0-9]', 'x')
}
$Share = 'C:\ProgramData\bdfq-share'     # readable by every user, writable by administrators only
New-Item -ItemType Directory -Path $Share -Force | Out-Null
& icacls.exe $Share /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' /Q | Out-Null
$Drop = 'C:\ProgramData\bdfq-drop'       # every user may write here (output files, stop files)
New-Item -ItemType Directory -Path $Drop -Force | Out-Null
& icacls.exe $Drop /grant '*S-1-5-32-545:(OI)(CI)M' /Q | Out-Null
$Users = @{}
function New-TestUser([string]$name) {
    $pw = New-Password
    New-LocalUser -Name $name -Password (ConvertTo-SecureString $pw -AsPlainText -Force) -PasswordNeverExpires -AccountNeverExpires | Out-Null
    Add-LocalGroupMember -Group 'Users' -Member $name
    $sid = (Get-LocalUser -Name $name).SID.Value
    $sb = New-Object System.Text.StringBuilder 260
    $hr = [BdfQ.Profile]::CreateProfile($sid, $name, $sb, 260)
    if ($hr -ne 0 -and $hr -ne -2147024713) { throw "CreateProfile for $name returned 0x{0:X8}" -f $hr }
    $cred = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$name", (ConvertTo-SecureString $pw -AsPlainText -Force))
    $Users[$name] = [pscustomobject]@{ Name = $name; Sid = $sid; Cred = $cred; Profile = (Join-Path $env:SystemDrive "Users\$name"); App = (Join-Path $env:SystemDrive "Users\$name\AppData\Local\Programs\BDF") }
    $Users[$name]
}
$PS = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
function Wait-Task([string]$Name, [int]$TimeoutSec) {
    $end = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $end) {
        Start-Sleep -Milliseconds 500
        $st = (Get-ScheduledTask -TaskName $Name).State
        $info = Get-ScheduledTaskInfo -TaskName $Name
        if ($st -eq 'Ready' -and $info.LastRunTime.Year -gt 2000) { return [int64]$info.LastTaskResult }
    }
    Stop-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
    -1
}
function Start-SystemTask([string]$ScriptPath, [string[]]$Arguments) {
    $name = 'BDFQ-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    $argLine = (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$ScriptPath`"") + $Arguments) -join ' '
    $a = New-ScheduledTaskAction -Execute $PS -Argument $argLine
    $p = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $s = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 40)
    Register-ScheduledTask -TaskName $name -Action $a -Principal $p -Settings $s | Out-Null
    Start-ScheduledTask -TaskName $name
    $name
}
function Remove-SystemTask([string]$Name) { Unregister-ScheduledTask -TaskName $Name -Confirm:$false -ErrorAction SilentlyContinue }
$DeployLog = 'C:\ProgramData\BDF\deploy.log'
function Log-Lines { if (Test-Path $DeployLog) { @(Get-Content $DeployLog).Count } else { 0 } }
# Runs the deployment script as Local System (what Action1 does) and returns its exit code and the lines it logged.
function Deploy-AsSystem([string]$Zip, [string]$Release, [string[]]$Extra = @(), [string]$Script = $Deploy, [int]$TimeoutSec = 900) {
    $before = Log-Lines
    $args2 = @('-PackageZip', "`"$Zip`"", '-ReleaseFile', "`"$Release`"", '-PublicKey', $Pub, '-Channel', 'dev') + $Extra
    $name = Start-SystemTask $Script $args2
    try { $code = Wait-Task $name $TimeoutSec } finally { Remove-SystemTask $name }
    $log = if (Test-Path $DeployLog) { (@(Get-Content $DeployLog) | Select-Object -Skip $before) -join "`n" } else { '' }
    [pscustomobject]@{ Code = $code; Log = $log }
}
# Runs a script as a person (a standard user): started by the machine's secondary-logon service with that person's credential.
function Start-AsUser($User, [string]$ScriptPath, [string[]]$Arguments = @(), [string]$OutFile) {
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$ScriptPath`"") + $Arguments
    $sp = @{ FilePath = $PS; Credential = $User.Cred; WorkingDirectory = (Join-Path $env:SystemRoot 'System32'); ArgumentList = $argv; PassThru = $true; WindowStyle = 'Hidden'; LoadUserProfile = $true }
    if ($OutFile) { $sp.RedirectStandardOutput = $OutFile; $sp.RedirectStandardError = "$OutFile.err" }
    $p = Start-Process @sp
    $null = $p.Handle
    $p
}
function Run-AsUser($User, [string]$ScriptText, [int]$TimeoutSec = 300) {
    $id = [guid]::NewGuid().ToString('N').Substring(0, 8)
    $script = Join-Path $Share "run-$id.ps1"
    Set-Content -Path $script -Value $ScriptText -Encoding utf8
    $out = Join-Path $Drop "out-$id.txt"
    New-Item -ItemType File -Path $out -Force | Out-Null
    & icacls.exe $out /grant '*S-1-5-32-545:(M)' /Q | Out-Null
    $p = Start-AsUser $User $script @() $out
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; return [pscustomobject]@{ Code = -1; Out = 'timed out' } }
    [pscustomobject]@{ Code = $p.ExitCode; Out = ((Get-Content $out -Raw -ErrorAction SilentlyContinue) + (Get-Content "$out.err" -Raw -ErrorAction SilentlyContinue)) }
}
function Owner-Of([string]$Path) { (Get-Acl -LiteralPath $Path).Owner }
function Version-Of([string]$dir) {
    $m = (Get-Content (Join-Path $dir 'bdf-package.json') -Raw -ErrorAction SilentlyContinue) | Select-String '"version": "([0-9.]+)"'
    if ($m) { $m.Matches[0].Groups[1].Value } else { '' }
}
function Pkg([string]$v, [string]$what) { Join-Path $Material (Join-Path "packages\$v" $(if ($what -eq 'zip') { "bdf-$v-windows-x64-portable.zip" } else { 'bdf-update.json' })) }
if ($Stage -ne 'probe') { $Zip0 = Pkg $V0 'zip'; $Rel0 = Pkg $V0 'rel'; $Zip1 = Pkg $V1 'zip'; $Rel1 = Pkg $V1 'rel' }
function Start-Feed {
    $script = Join-Path $PSScriptRoot 'preview-feed.ps1'
    $p = Start-Process -FilePath $PS -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$script`"", "`"$(Join-Path $Material 'feed')`"", "$Port") -PassThru -WindowStyle Hidden
    $end = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $end) { try { if ((Invoke-WebRequest -Uri "http://127.0.0.1:$Port/release.json" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200) { return $p } } catch { }; Start-Sleep -Milliseconds 300 }
    throw 'the local test feed did not start'
}
function Expand-Package([string]$Zip, [string]$Dest) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $tmp = Join-Path $Dest '.x'; New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $tmp)
    $top = Get-ChildItem $tmp -Directory | Select-Object -First 1
    Get-ChildItem $top.FullName -Force | Move-Item -Destination $Dest
    Remove-Item $tmp -Recurse -Force
}

# =====================================================================================================================
function Stage-System {
    Scenario 'Two real standard users, deployed to as Local System' {
        $script:q1 = New-TestUser 'bdfq1'; $script:q2 = New-TestUser 'bdfq2'
        Check (@(Get-LocalGroupMember -Group 'Administrators' | Where-Object { $_.Name -like '*\bdfq1' }).Count -eq 0) 'the test users are standard users (not administrators)'
        $r = Deploy-AsSystem $Zip0 $Rel0 @('-Users', 'bdfq1,bdfq2', '-IncludeDefaultProfile')
        Say ($r.Log -split "`n" | ForEach-Object { "      $_" } | Out-String).TrimEnd()
        Check ($r.Code -eq 0) "the Local System run exits 0 (it exited $($r.Code))"
        foreach ($u in $q1, $q2) {
            Check ((Version-Of $u.App) -eq $V0) "$($u.Name): BDF $V0 is installed in their own folder"
            Check ((Owner-Of (Join-Path $u.App 'bdf.exe')) -like "*\$($u.Name)") "$($u.Name): the files belong to that person, not to SYSTEM or Administrators (owner: $(Owner-Of (Join-Path $u.App 'bdf.exe')))"
            Check (-not (Test-Path (Join-Path $u.App 'BDFData'))) "$($u.Name): no data folder was invented"
        }
        Check (Test-Path 'C:\Users\Default\AppData\Local\Programs\BDF\bdf.exe') 'the Default profile has a copy, for users created later'
    }
    Scenario 'The person runs and writes in their own copy; another person cannot' {
        $a = Run-AsUser $q1 "`$app = '$($q1.App)'; `$s = & `"`$app\bdf-cli.exe`" update status | Out-String; Write-Output `$s; try { Set-Content -LiteralPath `"`$app\wrote.txt`" 'x'; Remove-Item -LiteralPath `"`$app\wrote.txt`"; Write-Output 'WRITE-OK' } catch { Write-Output 'WRITE-DENIED' }"
        Check ($a.Out -match '"managed": true') "bdfq1 runs their copy and it is a managed copy"
        Check ($a.Out -match 'WRITE-OK') 'bdfq1 can modify files in their own copy (so the in-app update needs no administrator)'
        $b = Run-AsUser $q2 "try { Set-Content -LiteralPath '$($q1.App)\intruder.txt' 'x'; Write-Output 'WRITE-OK' } catch { Write-Output 'WRITE-DENIED' }"
        Check ($b.Out -match 'WRITE-DENIED' -and -not (Test-Path (Join-Path $q1.App 'intruder.txt'))) 'bdfq2 cannot write into bdfq1''s copy'
    }
    Scenario 'A rerun changes nothing' {
        $stamp = (Get-Item (Join-Path $q1.App 'bdf.exe')).LastWriteTimeUtc
        $r = Deploy-AsSystem $Zip0 $Rel0 @('-Users', 'bdfq1,bdfq2')
        Check ($r.Code -eq 0 -and @($r.Log -split "`n" | Where-Object { $_ -match 'already has' }).Count -eq 2) 'both people are reported as already current'
        Check ((Get-Item (Join-Path $q1.App 'bdf.exe')).LastWriteTimeUtc -eq $stamp) 'the program file was not rewritten'
    }
    Scenario 'A user created later gets the copy from the Default profile, owns it, and updates it themselves from the feed' {
        $q3 = New-TestUser 'bdfq3'
        Check (Test-Path (Join-Path $q3.App 'bdf.exe')) 'bdfq3 has BDF in their profile from the first minute'
        if (Test-Path (Join-Path $q3.App 'bdf.exe')) {
            Check ((Owner-Of (Join-Path $q3.App 'bdf.exe')) -like '*\bdfq3') "the copied files belong to bdfq3 (owner: $(Owner-Of (Join-Path $q3.App 'bdf.exe')))"
            $w = Run-AsUser $q3 "try { Set-Content -LiteralPath '$($q3.App)\wrote.txt' 'x'; Remove-Item -LiteralPath '$($q3.App)\wrote.txt'; Write-Output 'WRITE-OK' } catch { Write-Output 'WRITE-DENIED' }"
            Check ($w.Out -match 'WRITE-OK') 'bdfq3 can modify their copy'
            $feed = Start-Feed
            try {
                $app = $q3.App
                $text = @"
`$env:BDF_UPDATE_FEED = 'http://127.0.0.1:$Port/release.json'
`$cli = '$app\bdf-cli.exe'
& `$cli update check | Out-String | Write-Output
& `$cli update download | Out-String | Write-Output
& `$cli update apply | Out-String | Write-Output
"@
                $u = Run-AsUser $q3 $text 600
                Say ($u.Out -split "`n" | Select-Object -First 12 | ForEach-Object { "      $_" } | Out-String).TrimEnd()
                $end = (Get-Date).AddSeconds(120)
                while ((Get-Date) -lt $end -and (Version-Of $app) -ne $V1) { Start-Sleep -Milliseconds 500 }
                Check ((Version-Of $app) -eq $V1) "bdfq3, a standard user, updated their own SYSTEM-deployed copy to $V1 from the feed (it is $(Version-Of $app))"
                $end = (Get-Date).AddSeconds(60); while ((Get-Date) -lt $end -and (Version-Of (Join-Path $app 'BDFPrevious')) -ne $V0) { Start-Sleep -Milliseconds 500 }
                Check ((Version-Of (Join-Path $app 'BDFPrevious')) -eq $V0) 'and BDFPrevious holds the version they had (the updater tidies up just after the swap)'
            } finally { Stop-Process -Id $feed.Id -Force -ErrorAction SilentlyContinue }
        }
    }
    Scenario 'A running copy is skipped, and updated by the next run' {
        $hold = Join-Path $Share 'hold.ps1'
        Set-Content -Path $hold -Encoding utf8 -Value "`$f = [IO.File]::Open('$($q1.App)\bdf-cli.exe', 'Open', 'Read', 'Read'); Start-Sleep -Seconds 120; `$f.Dispose()"
        $p = Start-AsUser $q1 $hold
        Start-Sleep -Seconds 4
        $r = Deploy-AsSystem $Zip1 $Rel1 @('-Users', 'bdfq1,bdfq2')
        Say ($r.Log -split "`n" | ForEach-Object { "      $_" } | Out-String).TrimEnd()
        Check ($r.Code -eq 0 -and $r.Log -match 'bdfq1: BDF is running from its folder') 'bdfq1 (a program running from the folder) is skipped, not failed, and the run exits 0'
        Check ((Version-Of $q1.App) -eq $V0) 'the running copy is unchanged'
        Check ((Version-Of $q2.App) -eq $V1) 'bdfq2 (nothing running) was updated in the same run'
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 2
        $r = Deploy-AsSystem $Zip1 $Rel1 @('-Users', 'bdfq1,bdfq2')
        Check ($r.Code -eq 0 -and (Version-Of $q1.App) -eq $V1) 'the next run updates bdfq1'
        Check ((Version-Of (Join-Path $q1.App 'BDFPrevious')) -eq $V0) 'bdfq1 keeps the version they had in BDFPrevious'
    }
    Scenario 'An account that may not run batch tasks is reported as failed and nothing is changed' {
        $q4 = New-TestUser 'bdfq4'
        $cfg = Join-Path $env:TEMP 'bdfq-rights.inf'; $db = Join-Path $env:TEMP 'bdfq-rights.sdb'
        & secedit.exe /export /cfg $cfg /areas USER_RIGHTS | Out-Null
        $text = Get-Content $cfg -Raw
        if ($text -match '(?m)^SeDenyBatchLogonRight\s*=.*$') { $text = $text -replace '(?m)^(SeDenyBatchLogonRight\s*=.*)$', "`$1,*$($q4.Sid)" } else { $text = $text -replace '(?m)^(\[Privilege Rights\])', "`$1`r`nSeDenyBatchLogonRight = *$($q4.Sid)" }
        Set-Content -Path $cfg -Value $text -Encoding Unicode
        & secedit.exe /configure /db $db /cfg $cfg /areas USER_RIGHTS | Out-Null
        $r = Deploy-AsSystem $Zip1 $Rel1 @('-Users', 'bdfq4,bdfq2')
        Say ($r.Log -split "`n" | ForEach-Object { "      $_" } | Out-String).TrimEnd()
        Check ($r.Code -eq 1 -and $r.Log -match 'bdfq4: FAILED') 'the person denied batch logon is reported as failed and the run exits 1'
        Check (-not (Test-Path $q4.App)) 'nothing was created in their profile'
        Check ($r.Log -match 'bdfq2: already has') 'the other person in the same run is unaffected'
        $why = (($r.Log -split "`n" | Where-Object { $_ -match 'bdfq4: FAILED' }) -join ' ')
        Info "the message names the cause: $why"
    }
    Scenario '-WhatIf reports and changes nothing' {
        $q5 = New-TestUser 'bdfq5'
        $r = Deploy-AsSystem $Zip0 $Rel0 @('-Users', 'bdfq5', '-WhatIf')
        Check ($r.Log -match 'would install' -and -not (Test-Path $q5.App)) '-WhatIf as Local System says what it would do and creates nothing'
    }
    Scenario 'A run ended part-way is finished by the next run, and clears what it left' {
        $q6 = New-TestUser 'bdfq6'
        $name = Start-SystemTask $Deploy @('-PackageZip', "`"$Zip0`"", '-ReleaseFile', "`"$Rel0`"", '-PublicKey', $Pub, '-Channel', 'dev', '-Users', 'bdfq6')
        Start-Sleep -Seconds 3
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue      # the deployment tool ends the run
        Start-Sleep -Seconds 1
        Remove-SystemTask $name
        $stale = @(Get-ChildItem 'C:\ProgramData\BDF' -Directory -Filter 'run-*' -ErrorAction SilentlyContinue)
        Info "after the run was ended: $($stale.Count) working folder(s) left, copy is '$(Version-Of $q6.App)'"
        Start-Sleep -Seconds 20     # the person's own task is a separate task: it finishes (or not) by itself
        foreach ($d in $stale) { $d.LastWriteTime = (Get-Date).AddHours(-3) }
        foreach ($t in @(Get-ScheduledTask -TaskPath '\BDF\' -ErrorAction SilentlyContinue)) { Info "task left from the ended run: $($t.TaskName) ($($t.State))" }
        $r = Deploy-AsSystem $Zip0 $Rel0 @('-Users', 'bdfq6')
        Say ($r.Log -split "`n" | ForEach-Object { "      $_" } | Out-String).TrimEnd()
        Check ($r.Code -eq 0 -and (Version-Of $q6.App) -eq $V0) 'the next run succeeds and the copy is complete'
        Check (@(Get-ChildItem 'C:\ProgramData\BDF' -Directory -Filter 'run-*' -ErrorAction SilentlyContinue).Count -eq 0) 'no working folder is left behind after the next run'
        Check (-not (Test-Path (Join-Path $q6.App 'BDFUpdate\deploy-pending.json'))) 'no unfinished-run marker is left in the person''s folder'
    }
}

# =====================================================================================================================
function Stage-Boundary {
    # Synthetic packages with many files (so there is a window to hit), signed with a throwaway key made here.
    $key = Join-Path $env:TEMP 'bdfq-boundary.key'
    $keygen = & cargo run --locked -q -p xtask -- update-keygen --out $key | Out-String
    $bpub = (($keygen -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '').Trim()
    $models = Join-Path $env:TEMP 'bdfq-models'; New-Item -ItemType Directory -Path (Join-Path $models 'models') -Force | Out-Null
    $bin = Join-Path $env:TEMP 'bdfq-fakebin'; New-Item -ItemType Directory -Path $bin -Force | Out-Null
    function Make-Rel([string]$v) {
        foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe') { Set-Content -Path (Join-Path $bin $n) -Value "$n $v" -Encoding ascii }
        foreach ($i in 1..60) { Set-Content -Path (Join-Path $models ('models\m{0:000}.bin' -f $i)) -Value "model $i of $v" -Encoding ascii }
        $out = Join-Path $env:TEMP "bdfq-rel-$v"
        & cargo run --locked -q -p xtask -- portable-package --bin-dir $bin --models-dir $models --version $v --out $out | Out-Null
        & cargo run --locked -q -p xtask -- update-sign --zip (Join-Path $out "bdf-$v-windows-x64-portable.zip") --version $v --key $key --out $out --channel dev --url-base "http://127.0.0.1:1/v$v" | Out-Null
        [pscustomobject]@{ Zip = (Join-Path $out "bdf-$v-windows-x64-portable.zip"); Rel = (Join-Path $out 'bdf-update.json') }
    }
    $a = Make-Rel '0.8.1'; $b = Make-Rel '0.8.2'
    $outside = 'C:\ProgramData\bdfq-outside'    # administrators only: a person cannot touch what is in here
    New-Item -ItemType Directory -Path $outside -Force | Out-Null
    & icacls.exe $outside /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /Q | Out-Null
    $attacker = Join-Path $Share 'attacker.ps1'
    Set-Content -Path $attacker -Encoding utf8 -Value @'
param([string]$App, [string]$Outside, [string]$Stop)
$dir = Join-Path $App 'models'; $real = Join-Path $App 'models.real'
$gate = Join-Path $App 'BDFUpdate\deploy-stage'
$until = [DateTime]::UtcNow.AddSeconds(90)
while (-not (Test-Path -LiteralPath $gate) -and [DateTime]::UtcNow -lt $until -and -not (Test-Path $Stop)) { }
while (-not (Test-Path $Stop)) {
    try { [IO.Directory]::Move($dir, $real); try { New-Item -ItemType Junction -Path $dir -Target $Outside | Out-Null } catch { }; [IO.Directory]::Delete($dir) } catch { }
    try { if (-not (Test-Path -LiteralPath $dir) -and (Test-Path -LiteralPath $real)) { [IO.Directory]::Move($real, $dir) } } catch { }
}
try { if ((Test-Path -LiteralPath $dir) -and ((Get-Item -LiteralPath $dir -Force).Attributes -band 'ReparsePoint')) { [IO.Directory]::Delete($dir) } } catch { }
try { if (-not (Test-Path -LiteralPath $dir) -and (Test-Path -LiteralPath $real)) { [IO.Directory]::Move($real, $dir) } } catch { }
'@
    function Race([string]$Label, [string]$Script, [string]$UserName, [bool]$Assert) {
        $u = New-TestUser $UserName
        $hits = 0; $ran = 0; $updated = 0
        foreach ($t in 1..$Trials) {
            # a fresh outside folder of canaries, and a fresh installed older copy
            Get-ChildItem $outside -Force | Remove-Item -Recurse -Force
            foreach ($i in 1..60) { Set-Content -Path (Join-Path $outside ('m{0:000}.bin' -f $i)) -Value "CANARY $i" -Encoding ascii }
            if (Test-Path $u.App) { Remove-Item $u.App -Recurse -Force }
            $r0 = Deploy-AsSystem $a.Zip $a.Rel @('-Users', $UserName) $Script
            $events = [System.Collections.Concurrent.ConcurrentBag[string]]::new()
            $watch = New-Object System.IO.FileSystemWatcher $outside
            $watch.IncludeSubdirectories = $true; $watch.InternalBufferSize = 65536
            $watch.NotifyFilter = 'FileName, DirectoryName, LastWrite, Size, CreationTime, Attributes'
            $sink = { $Event.MessageData.Add($Event.SourceEventArgs.ChangeType.ToString() + ' ' + $Event.SourceEventArgs.Name) }
            $subs = foreach ($n in 'Created', 'Deleted', 'Changed', 'Renamed') { Register-ObjectEvent -InputObject $watch -EventName $n -Action $sink -MessageData $events }
            $watch.EnableRaisingEvents = $true
            $stopFile = Join-Path $Drop "stop-$UserName-$t"
            if (Test-Path $stopFile) { Remove-Item $stopFile }
            $ap = Start-AsUser $u $attacker @('-App', "`"$($u.App)`"", '-Outside', "`"$outside`"", '-Stop', "`"$stopFile`"")
            $r = Deploy-AsSystem $b.Zip $b.Rel @('-Users', $UserName) $Script
            Set-Content $stopFile 'stop'
            [void]$ap.WaitForExit(30000); if (-not $ap.HasExited) { Stop-Process -Id $ap.Id -Force -ErrorAction SilentlyContinue }
            Start-Sleep -Milliseconds 500
            $watch.EnableRaisingEvents = $false
            foreach ($s in $subs) { Unregister-Event -SubscriptionId $s.Id -ErrorAction SilentlyContinue }
            $watch.Dispose()
            $ran++
            if ($r.Log -match 'updated 0.8.1 -> 0.8.2') { $updated++ }
            if ($events.Count -gt 0) { $hits++; Info "$Label trial ${t}: steered through the link: $((@($events.ToArray()) | Select-Object -First 3) -join '; ')" }
        }
        Info "$Label`: $hits of $ran trials touched the administrators-only folder; $updated of $ran runs finished the update under attack"
        if ($Assert) { Check ($hits -eq 0) "$Label`: in $ran trials with a second process swapping a folder for a junction, nothing in the administrators-only folder was ever touched" }
        # the person's copy is healed by a run with nobody attacking
        $r = Deploy-AsSystem $b.Zip $b.Rel @('-Users', $UserName) $Script
        if ($Assert) { Check ($r.Code -eq 0 -and (Version-Of $u.App) -eq '0.8.2') "$Label`: with nobody attacking, the next run leaves a complete 0.8.2" }
        $hits
    }
    Scenario 'The previous in-place script, measured (informational: zero hits in a sample proves nothing)' {
        if ($LegacyDeploy -and (Test-Path $LegacyDeploy)) { [void](Race 'previous script' $LegacyDeploy 'bdfqold' $false) } else { Info 'the previous script was not supplied (-LegacyDeploy), so it was not measured here' }
    }
    Scenario 'The current script: the install runs as the person' {
        [void](Race 'current script' $Deploy 'bdfqnew' $true)
    }
}

# =====================================================================================================================
function New-SmallVolume([string]$Letter, [int]$MB) {
    $vhd = Join-Path $env:TEMP "bdfq-$Letter.vhd"
    $dp = Join-Path $env:TEMP "bdfq-$Letter.txt"
    Set-Content -Path $dp -Encoding ascii -Value @("create vdisk file=`"$vhd`" maximum=$MB type=fixed", 'attach vdisk', 'create partition primary', 'format fs=ntfs quick label=BDFQ', "assign letter=$Letter")
    & diskpart.exe /s $dp | Out-Null
    Start-Sleep -Seconds 3
    if (-not (Test-Path "${Letter}:\")) { throw "the ${MB} MB volume ${Letter}: did not appear" }
}
function Fill-To([string]$Letter, [int64]$FreeBytes) {
    $fill = "${Letter}:\fill.bin"
    if (Test-Path $fill) { Remove-Item $fill -Force }
    $free = ([IO.DriveInfo]"${Letter}:").AvailableFreeSpace
    if ($free -gt $FreeBytes) { & fsutil.exe file createnew $fill ($free - $FreeBytes) | Out-Null }
    ([IO.DriveInfo]"${Letter}:").AvailableFreeSpace
}
function Tree-State([string]$Dir) {
    $r = @{}
    foreach ($f in Get-ChildItem -LiteralPath $Dir -Recurse -File -Force -ErrorAction SilentlyContinue) {
        $rel = $f.FullName.Substring($Dir.Length + 1).ToLowerInvariant()
        if ($rel -match '^(bdfdata|bdfupdate|bdfprevious)\\') { continue }
        $r[$rel] = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash
    }
    $r
}
function Same-State($A, $B) { ($A.Keys.Count -eq $B.Keys.Count) -and -not @($A.Keys | Where-Object { $B[$_] -ne $A[$_] }) }
function Stage-Disk {
    $letter = 'Z'
    Scenario 'A 900 MB volume' { New-SmallVolume $letter 900; Info ('the volume has {0:n0} MB free' -f (([IO.DriveInfo]"${letter}:").AvailableFreeSpace / 1MB)) }
    $zipBytes = (Get-Item $Zip1).Length
    Info ('the update package is {0:n0} MB' -f ($zipBytes / 1MB))
    $feed = Start-Feed
    try {
        function Fresh-App([string]$Name) {
            if (Test-Path "${letter}:\fill.bin") { Remove-Item "${letter}:\fill.bin" -Force }
            foreach ($old in @(Get-ChildItem "${letter}:\" -Directory -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -ne 'System Volume Information' -and $_.Name -ne '$RECYCLE.BIN' })) { Remove-Item $old.FullName -Recurse -Force -ErrorAction SilentlyContinue }
            $app = "${letter}:\$Name"
            New-Item -ItemType Directory -Path $app | Out-Null
            Expand-Package $Zip0 $app
            New-Item -ItemType Directory -Path "$app\BDFData\recovery" -Force | Out-Null
            Set-Content "$app\BDFData\settings.json" '{"theme_choice":"dark"}'
            Set-Content "$app\BDFData\recovery\doc.autosave" 'unsaved work'
            $app
        }
        function BdfCli([string]$App, [string[]]$a) {
            $env:BDF_UPDATE_FEED = "http://127.0.0.1:$Port/release.json"
            $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
            try { $o = & (Join-Path $App 'bdf-cli.exe') @a 2>&1 | Out-String; $code = $LASTEXITCODE } finally { $ErrorActionPreference = $prev; Remove-Item Env:\BDF_UPDATE_FEED -ErrorAction SilentlyContinue }
            [pscustomobject]@{ Code = $code; Out = $o }
        }
        function Data-Hash([string]$App) { (Get-FileHash "$App\BDFData\settings.json").Hash + (Get-FileHash "$App\BDFData\recovery\doc.autosave").Hash }

        Scenario 'Enough room: the baseline update works on this volume' {
            $app = Fresh-App 'ok'
            [void](Fill-To $letter 400MB)
            $c = BdfCli $app @('update', 'check'); $d = BdfCli $app @('update', 'download'); $ap = BdfCli $app @('update', 'apply')
            $end = (Get-Date).AddSeconds(120); while ((Get-Date) -lt $end -and (Version-Of $app) -ne $V1) { Start-Sleep -Milliseconds 500 }
            Check ((Version-Of $app) -eq $V1) "with 400 MB free the update completes (check $($c.Code), download $($d.Code), apply $($ap.Code))"
        }
        Scenario 'No room for the download' {
            $app = Fresh-App 'nodl'
            $before = Tree-State $app; $dh = Data-Hash $app
            $free = Fill-To $letter 20MB
            Info ('free space now {0:n1} MB (the package is {1:n1} MB)' -f ($free / 1MB), ($zipBytes / 1MB))
            [void](BdfCli $app @('update', 'check'))
            $d = BdfCli $app @('update', 'download')
            Say ($d.Out.Trim() -split "`n" | Select-Object -First 4 | ForEach-Object { "      $_" } | Out-String).TrimEnd()
            Check ($d.Code -ne 0) "the download fails, and says so (exit $($d.Code))"
            $st = BdfCli $app @('update', 'status')
            Check ($st.Out -notmatch '"ready_to_install": true') 'a partial download is never offered as ready to install'
            Check ((Same-State $before (Tree-State $app)) -and (Data-Hash $app) -eq $dh) 'the program files and the person''s data are exactly as they were'
            $leftover = @(Get-ChildItem "$app\BDFUpdate" -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -like '*.zip*' -or $_.Name -like '*.part*' })
            Info "files left in BDFUpdate after the failed download: $($leftover.Count)"
            [void](Fill-To $letter 400MB)
            $c = BdfCli $app @('update', 'check'); $d = BdfCli $app @('update', 'download')
            if ($d.Code -ne 0) { Say ("      check: $($c.Out.Trim() -replace '\s+', ' ')"); Say ("      download: $($d.Out.Trim() -replace '\s+', ' ')") }
            Check ($d.Code -eq 0) 'with room again the same download succeeds'
        }
        Scenario 'Room for the download, none for the replacement' {
            $app = Fresh-App 'noapply'
            $before = Tree-State $app; $dh = Data-Hash $app
            [void](Fill-To $letter 400MB)
            [void](BdfCli $app @('update', 'check')); $d = BdfCli $app @('update', 'download')
            Check ($d.Code -eq 0) 'the package downloads and verifies'
            $free = Fill-To $letter 40MB
            Info ('free space now {0:n1} MB (the staged copy needs about {1:n1} MB)' -f ($free / 1MB), ((Get-ChildItem $app -Recurse -File -Force | Where-Object { $_.FullName -notmatch 'BDFUpdate|BDFData' } | Measure-Object Length -Sum).Sum / 1MB))
            $ap = BdfCli $app @('update', 'apply')
            Start-Sleep -Seconds 25
            Check ((Version-Of $app) -eq $V0) "the copy is still $V0 (apply exit $($ap.Code))"
            Check ((Same-State $before (Tree-State $app)) -and (Data-Hash $app) -eq $dh) 'the program files and the person''s data are exactly as they were'
            Check (-not (Test-Path "$app\BDFUpdate\journal.json")) 'no half-finished journal is left'
            $res = Get-Content "$app\BDFUpdate\last-result.json" -Raw -ErrorAction SilentlyContinue
            Info "last result: $(if ($res) { $res.Trim() -replace '\s+', ' ' } else { '(none written: no room even for that)' })"
            $st = BdfCli $app @('update', 'status')
            Info "the copy still starts: $($st.Code -eq 0)"
            [void](Fill-To $letter 400MB)
            $ap = BdfCli $app @('update', 'apply')
            $end = (Get-Date).AddSeconds(120); while ((Get-Date) -lt $end -and (Version-Of $app) -ne $V1) { Start-Sleep -Milliseconds 500 }
            Check ((Version-Of $app) -eq $V1) 'with room again the update completes'
        }
        Scenario 'A completely full volume' {
            $app = Fresh-App 'full'
            $before = Tree-State $app
            [void](Fill-To $letter 0)
            $st = BdfCli $app @('update', 'status')
            Check ($st.Code -eq 0) 'the program still starts and reports status on a full volume'
            $p = Start-Process -FilePath (Join-Path $app 'bdf.exe') -PassThru
            Start-Sleep -Seconds 12
            Check (-not $p.HasExited) 'BDF itself starts and stays up on a full volume (it cannot write its settings or log, and does not crash)'
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
            Check (Same-State $before (Tree-State $app)) 'the program files are unchanged'
            [void](Fill-To $letter 400MB)
        }
        Scenario 'The deployment script on a nearly full volume' {
            $users = "${letter}:\Users"; if (Test-Path $users) { Remove-Item $users -Recurse -Force }
            New-Item -ItemType Directory -Path "$users\vic" | Out-Null
            [void](Fill-To $letter 400MB)
            $r0 = Deploy-AsSystem $Zip0 $Rel0 @('-UsersRoot', $users, '-Users', 'vic')
            Check ($r0.Code -eq 0 -and (Version-Of "$users\vic\AppData\Local\Programs\BDF") -eq $V0) 'baseline: the script installs on this volume with room'
            $app = "$users\vic\AppData\Local\Programs\BDF"
            $before = Tree-State $app
            New-Item -ItemType Directory -Path "$app\BDFData" -Force | Out-Null; Set-Content "$app\BDFData\settings.json" 'mine'
            [void](Fill-To $letter 30MB)
            $r = Deploy-AsSystem $Zip1 $Rel1 @('-UsersRoot', $users, '-Users', 'vic')
            Say ($r.Log -split "`n" | ForEach-Object { "      $_" } | Out-String).TrimEnd()
            Check ($r.Code -eq 1 -and $r.Log -match 'vic: FAILED') "the update fails and says so (exit $($r.Code))"
            Check ((Same-State $before (Tree-State $app)) -and (Get-Content "$app\BDFData\settings.json") -eq 'mine') 'the copy is exactly the old version and the data is untouched'
            Check (-not (Test-Path "$app\BDFUpdate\deploy-pending.json")) 'no unfinished-run marker is left (nothing had been displaced)'
            [void](Fill-To $letter 400MB)
            $r = Deploy-AsSystem $Zip1 $Rel1 @('-UsersRoot', $users, '-Users', 'vic')
            Check ($r.Code -eq 0 -and (Version-Of $app) -eq $V1) 'with room again the next run updates it'
        }
    } finally { Stop-Process -Id $feed.Id -Force -ErrorAction SilentlyContinue }
}

# =====================================================================================================================
function Stage-Desktop {
    Scenario 'The real window: a complete update' {
        if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { Info 'not run on ARM64 here: this window test drives the x64 build (under emulation on ARM64); it is qualified on x64'; return }
        $work = Join-Path $env:TEMP 'bdfq-desktop'
        if (Test-Path $work) { Remove-Item $work -Recurse -Force }
        $app = Join-Path $work 'BDF'; New-Item -ItemType Directory -Path $app -Force | Out-Null
        Expand-Package $Zip0 $app
        $gui = Join-Path $PSScriptRoot 'test-update-gui.ps1'
        $shots = Join-Path $EvidenceDir 'desktop-shots'
        $out = & $PS -NoProfile -ExecutionPolicy Bypass -File $gui -BdfDir $app -FeedDir (Join-Path $Material 'feed') -FromVersion $V0 -ToVersion $V1 -Port $Port -EvidenceDir $shots 2>&1 | Out-String
        foreach ($l in ($out -split "`r?`n")) { if ($l -match '^(ok|FAIL|all|[0-9]+ check)') { Say $l } else { if ($l.Trim()) { Info $l.Trim() } } }
        if ($out -match '(?m)^FAIL') { $script:failures++ } elseif ($out -notmatch 'all GUI update checks passed') { Say 'FAIL  the window test did not complete'; $script:failures++ }
    }
    Scenario 'Windows integration: registration is visible to Windows' {
        $work = Join-Path $env:TEMP 'bdfq-integration'
        if (Test-Path $work) { Remove-Item $work -Recurse -Force }
        $app = Join-Path $work 'BDF'; New-Item -ItemType Directory -Path $app -Force | Out-Null
        Expand-Package $Zip0 $app
        $cli = Join-Path $app 'bdf-cli.exe'
        $before = & $cli windows status | Out-String
        Check ($before -match '"registered": false') 'before: this copy is not registered'
        [void](& $cli windows register)
        $after = & $cli windows status | Out-String
        Check ($after -match '"registered": true' -and $after -match '"this_copy": true') 'after register: this copy is registered'
        Info "Windows' own answer for what opens .pdf now: $(Assoc-Command '.pdf')"
        $progIds = @(Get-ItemProperty 'HKCU:\Software\Classes\.pdf\OpenWithProgids' -ErrorAction SilentlyContinue | Get-Member -MemberType NoteProperty | Where-Object { $_.Name -notlike 'PS*' } | ForEach-Object Name)
        Info "ProgIDs offered for .pdf in this account: $($progIds -join ', ')"
        $reg = Get-ItemProperty 'HKCU:\Software\RegisteredApplications' -ErrorAction SilentlyContinue
        Check ([bool]($reg -and ($reg.PSObject.Properties | Where-Object { $_.Name -like '*BDF*' }))) 'BDF is listed under RegisteredApplications for this account (what the Default apps page reads)'
        [void](& $cli windows unregister)
        Check ((& $cli windows status | Out-String) -match '"registered": false') 'unregister removes it again'
    }
    Scenario 'The default-app choice in Windows Settings, then a PDF double-click' {
        Info 'Windows does not let a program choose itself as the default for .pdf: the choice is the person''s, made in Settings > Default apps.'
        Info 'Here the real Settings page for the registered app is opened and driven through UI Automation (what an assistive technology does),'
        Info 'then Windows is asked what opens .pdf, and a .pdf is opened the way a double-click does (ShellExecute, default verb).'
        Info ("operating system: {0} (build {1}), architecture {2}" -f (Get-CimInstance Win32_OperatingSystem).Caption, [Environment]::OSVersion.Version.Build, $env:PROCESSOR_ARCHITECTURE)
        $work = Join-Path $env:TEMP 'bdfq-integration'
        $app = Join-Path $work 'BDF'
        $cli = Join-Path $app 'bdf-cli.exe'
        [void](& $cli windows register)
        $pdf = Join-Path $work 'double-click.pdf'
        [IO.File]::WriteAllBytes($pdf, [Text.Encoding]::ASCII.GetBytes("%PDF-1.4`n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj`n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj`n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]>>endobj`ntrailer<</Root 1 0 R/Size 4>>`n%%EOF`n"))
        Info "before: Windows says .pdf opens with $(Assoc-Command '.pdf')"
        Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
        $ae = [System.Windows.Automation.AutomationElement]; $ts = [System.Windows.Automation.TreeScope]
        function Shot-Desktop([string]$name) {
            try {
                $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
                $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
                $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size); $g.Dispose()
                $bmp.Save((Join-Path $EvidenceDir "$name.png")); $bmp.Dispose()
            } catch { Info "could not take the desktop picture $name`: $($_.Exception.Message)" }
        }
        Start-Process 'ms-settings:defaultapps?registeredAppUser=BDF%20Portable'
        $win = $null
        $end = (Get-Date).AddSeconds(30)
        while (-not $win -and (Get-Date) -lt $end) {
            Start-Sleep -Seconds 1
            $win = $ae::RootElement.FindFirst($ts::Children, (New-Object System.Windows.Automation.PropertyCondition($ae::NameProperty, 'Settings')))
        }
        Check ([bool]$win) 'the Settings window for the registered app opens'
        Shot-Desktop 'default-app-1-settings'
        if ($win) {
            Start-Sleep -Seconds 3
            $names = @($win.FindAll($ts::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { '{0} | {1}' -f $_.Current.ControlType.ProgrammaticName, $_.Current.Name } | Where-Object { $_ -notmatch '\| $' } | Select-Object -Unique)
            Set-Content -Path (Join-Path $EvidenceDir 'default-app-settings-elements.txt') -Value $names -Encoding utf8
            Info "elements Settings exposes: $($names.Count) named (listed in default-app-settings-elements.txt)"
            function Dump-Elements([string]$file) {
                $all = @($ae::RootElement.FindAll($ts::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object {
                    try { '{0} | {1}' -f $_.Current.ControlType.ProgrammaticName, $_.Current.Name } catch { } } | Where-Object { $_ -and $_ -notmatch '\| $' } | Select-Object -Unique)
                Set-Content -Path (Join-Path $EvidenceDir $file) -Value $all -Encoding utf8
                $all
            }
            # On this page the file type is a row: ".pdf, <current app>, <its description>". Choosing the row offers the apps that can open it.
            $row = $win.FindFirst($ts::Descendants, (New-Object System.Windows.Automation.AndCondition(
                (New-Object System.Windows.Automation.PropertyCondition($ae::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)),
                (New-Object System.Windows.Automation.PropertyCondition($ae::IsEnabledProperty, $true)))))
            $rows = @($win.FindAll($ts::Descendants, (New-Object System.Windows.Automation.PropertyCondition($ae::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button))) | Where-Object { $_.Current.Name -like '.pdf*' })
            if ($rows.Count -eq 0) {
                Say 'PENDING  the .pdf row was not found on the Settings page (see default-app-settings-elements.txt)'
            } else {
                $beforeNames = @(Dump-Elements 'default-app-settings-elements.txt')
                Info "choosing the row '$($rows[0].Current.Name)'"
                try { $rows[0].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() } catch { Info "invoke failed: $($_.Exception.Message)" }
                Start-Sleep -Seconds 4
                Shot-Desktop 'default-app-2-chooser'
                $afterNames = @(Dump-Elements 'default-app-chooser-elements.txt')
                $newBdf = @($afterNames | Where-Object { $_ -match 'BDF' -and $beforeNames -notcontains $_ })
                Info "names with BDF in them that appeared after choosing the row: $($newBdf -join ' ;; ')"
                $choice = $null
                foreach ($e in $ae::RootElement.FindAll($ts::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
                    try { if ($e.Current.Name -match 'BDF' -and $e.Current.ControlType -ne [System.Windows.Automation.ControlType]::Window -and $beforeNames -notcontains ('{0} | {1}' -f $e.Current.ControlType.ProgrammaticName, $e.Current.Name)) { $choice = $e; break } } catch { }
                }
                if (-not $choice) {
                    Say 'PENDING  BDF was not offered in the chooser that the .pdf row opens (see default-app-chooser-elements.txt)'
                } else {
                    Info "choosing '$($choice.Current.Name)'"
                    $done = $false
                    foreach ($pattern in [System.Windows.Automation.SelectionItemPattern]::Pattern, [System.Windows.Automation.InvokePattern]::Pattern) {
                        try { $choice.GetCurrentPattern($pattern).GetType() | Out-Null; if ($pattern -eq [System.Windows.Automation.SelectionItemPattern]::Pattern) { $choice.GetCurrentPattern($pattern).Select() } else { $choice.GetCurrentPattern($pattern).Invoke() }; $done = $true; break } catch { }
                    }
                    Start-Sleep -Seconds 2
                    # a confirmation button, if the chooser has one
                    foreach ($label in 'Set default', 'OK', 'Select') {
                        $ok = $ae::RootElement.FindFirst($ts::Descendants, (New-Object System.Windows.Automation.AndCondition(
                            (New-Object System.Windows.Automation.PropertyCondition($ae::NameProperty, $label)),
                            (New-Object System.Windows.Automation.PropertyCondition($ae::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)))))
                        if ($ok) { Info "confirming with '$label'"; try { $ok.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() } catch { }; break }
                    }
                    Start-Sleep -Seconds 4
                    Shot-Desktop 'default-app-3-after-choice'
                }
            }            Get-Process -Name 'SystemSettings' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        }
        $after = Assoc-Command '.pdf'
        Info "after: Windows says .pdf opens with $after"
        if ($after -like "*$($app.Replace('\', '\\'))*" -or $after -like "*$app*") {
            Check $true 'Windows now says BDF opens .pdf'
            $t0 = Get-Date
            Start-Process -FilePath $pdf       # what a double-click does
            $ok = $false
            $end = (Get-Date).AddSeconds(40)
            while (-not $ok -and (Get-Date) -lt $end) {
                Start-Sleep -Seconds 1
                $ok = [bool](Get-Process -Name 'bdf' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($app, [StringComparison]::OrdinalIgnoreCase) -and $_.StartTime -gt $t0 })
            }
            Check $ok 'opening the .pdf (a double-click) started BDF from this copy'
            Start-Sleep -Seconds 6
            $title = (Get-Process -Name 'bdf' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($app, [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { $_.Refresh(); $_.MainWindowTitle }) -join ' | '
            Check ($title -match 'double-click\.pdf') "BDF opened that document (window title: $title)"
            Shot-Desktop 'default-app-3-double-click'
            Get-Process -Name 'bdf' -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($app, [StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force -ErrorAction SilentlyContinue
        } else {
            Say 'PENDING  the default-app choice could not be made without a person on this machine, so the double-click was not run (it needs a Windows 11 desktop and a click)'
        }
        [void](& $cli windows unregister)
    }
}

# =====================================================================================================================
function Stage-Probe {
    Scenario 'Acting as another person from Local System: what works, with the exact errors' {
        $u = New-TestUser 'bdfqp'
        $outside = 'C:\ProgramData\bdfq-outside'
        New-Item -ItemType Directory -Path $outside -Force | Out-Null
        & icacls.exe $outside /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /Q | Out-Null
        $out = Join-Path $Share 'probe-out.txt'
        if (Test-Path $out) { Remove-Item $out }
        $name = Start-SystemTask (Join-Path $PSScriptRoot 'qualify-probe-system.ps1') @('-User', 'bdfqp', '-Out', "`"$out`"", '-Shared', "`"$Drop`"", '-Admins', "`"$outside`"")
        $code = Wait-Task $name 300
        Remove-SystemTask $name
        Say "the probe ended with task result $code"
        if (Test-Path $out) { foreach ($l in Get-Content $out) { Say "      $l" } } else { Say 'FAIL  the probe wrote nothing'; $script:failures++ }
    }
}
try {
    switch ($Stage) {
        'system'   { Stage-System }
        'boundary' { Stage-Boundary }
        'disk'     { Stage-Disk }
        'desktop'  { Stage-Desktop }
        'probe'    { Stage-Probe }
    }
} finally {
    Say ''
    if ($script:failures) { Say "RESULT: $($script:failures) check(s) failed in stage '$Stage'" } else { Say "RESULT: every check passed in stage '$Stage'" }
}
exit $(if ($script:failures) { 1 } else { 0 })
