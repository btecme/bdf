<#
.SYNOPSIS
  Run as Local System on a disposable machine by qualify-windows.ps1 -Stage probe: which ways of acting as another person work from Local System.
.DESCRIPTION
  Writes what each attempt did (exact error, result code, who it ran as) to -Out. Nothing here is part of the product; it exists to choose
  the mechanism deploy-bdf.ps1 uses to run an install with a person's own rights.
#>
param([Parameter(Mandatory)][string]$User, [Parameter(Mandatory)][string]$Out, [Parameter(Mandatory)][string]$Shared, [Parameter(Mandatory)][string]$Admins)
$ErrorActionPreference = 'Continue'
function Log([string]$m) { Add-Content -Path $Out -Value $m -Encoding utf8 }
function Describe([System.Management.Automation.ErrorRecord]$e) {
    $x = $e.Exception
    $inner = if ($x.InnerException) { " / inner: $($x.InnerException.GetType().Name): $($x.InnerException.Message.Trim())" } else { '' }
    "{0}: {1} (HResult 0x{2:X8}){3}" -f $x.GetType().Name, $x.Message.Trim(), $x.HResult, $inner
}
Log ("running as: " + [Security.Principal.WindowsIdentity]::GetCurrent().Name)
$account = "$env:COMPUTERNAME\$User"
$sid = (Get-LocalUser -Name $User).SID.Value
$profile = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$sid").ProfileImagePath
Log "target: $account ($sid), profile $profile"

function Try-Task([string]$Label, [string]$Path, $Principal) {
    $name = 'BDFPROBE-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
    $marker = Join-Path $Shared "$name.txt"
    $action = New-ScheduledTaskAction -Execute (Join-Path $env:SystemRoot 'System32\cmd.exe') -Argument "/c whoami > `"$marker`" & exit 5"
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
    try { Register-ScheduledTask -TaskName $name -TaskPath $Path -Action $action -Principal $Principal -Settings $settings -ErrorAction Stop | Out-Null }
    catch { Log "$Label`: REGISTER failed: $(Describe $_)"; return }
    try { Start-ScheduledTask -TaskName $name -TaskPath $Path -ErrorAction Stop }
    catch { Log "$Label`: registered, START failed: $(Describe $_)"; Unregister-ScheduledTask -TaskName $name -TaskPath $Path -Confirm:$false -ErrorAction SilentlyContinue; return }
    $end = (Get-Date).AddSeconds(40); $info = $null
    while ((Get-Date) -lt $end) {
        Start-Sleep -Milliseconds 500
        $st = (Get-ScheduledTask -TaskName $name -TaskPath $Path).State
        $info = Get-ScheduledTaskInfo -TaskName $name -TaskPath $Path
        if ($st -eq 'Ready' -and $info.LastRunTime.Year -gt 2000) { break }
        $info = $null
    }
    if ($info) {
        $who = if (Test-Path $marker) { (Get-Content $marker -Raw).Trim() } else { '(no output)' }
        Log ("{0}: ran. task result {1} (5 means the command ran and exited 5), whoami = {2}" -f $Label, $info.LastTaskResult, $who)
    } else { Log "$Label`: started but did not finish in 40 s" }
    Unregister-ScheduledTask -TaskName $name -TaskPath $Path -Confirm:$false -ErrorAction SilentlyContinue
}

Log ''
Log '--- scheduled task as that person, no password (S4U), as Local System'
Try-Task 'S4U, task folder \BDF\' '\BDF\' (New-ScheduledTaskPrincipal -UserId $account -LogonType S4U -RunLevel Limited)
Try-Task 'S4U, root task folder' '\' (New-ScheduledTaskPrincipal -UserId $account -LogonType S4U -RunLevel Limited)
Log ''
Log "--- the same after adding $User to 'Performance Log Users' (a group that holds 'Log on as a batch job' by default)"
try {
    Add-LocalGroupMember -Group 'Performance Log Users' -Member $User -ErrorAction Stop
    Try-Task 'S4U with the batch right' '\BDF\' (New-ScheduledTaskPrincipal -UserId $account -LogonType S4U -RunLevel Limited)
} catch { Log "could not add to the group: $(Describe $_)" }
Remove-LocalGroupMember -Group 'Performance Log Users' -Member $User -ErrorAction SilentlyContinue
Log ''
Log '--- a task for the Users group (runs in a signed-in member''s own session)'
Try-Task 'group BUILTIN\Users, interactive' '\BDF\' (New-ScheduledTaskPrincipal -GroupId 'BUILTIN\Users' -RunLevel Limited)

Log ''
Log '--- impersonating that person inside this Local System process (a logon with no password: S4U through WindowsIdentity)'
foreach ($form in "$User@$env:COMPUTERNAME", $account, $User) {
    try {
        $id = New-Object System.Security.Principal.WindowsIdentity($form)
        Log "WindowsIdentity('$form'): token for $($id.Name), impersonation level $($id.ImpersonationLevel), authentication type $($id.AuthenticationType)"
        $ctx = $id.Impersonate()
        try {
            $mine = Join-Path $profile 'qualify-probe.txt'
            try { Set-Content -LiteralPath $mine -Value 'x' -ErrorAction Stop; $o = (Get-Acl -LiteralPath $mine).Owner; Log "  inside the person's profile: created a file, owner $o"; Remove-Item -LiteralPath $mine -Force } catch { Log "  inside the person's profile: could not create a file: $($_.Exception.Message.Trim())" }
            $theirs = Join-Path $Admins 'qualify-probe.txt'
            try { Set-Content -LiteralPath $theirs -Value 'x' -ErrorAction Stop; Log '  inside an administrators-only folder: WROTE A FILE (the boundary does not hold)'; Remove-Item -LiteralPath $theirs -Force } catch { Log "  inside an administrators-only folder: denied, as it should be ($($_.Exception.GetType().Name))" }
        } finally { $ctx.Undo() }
        break
    } catch { Log "WindowsIdentity('$form'): failed: $(Describe $_)" }
}
Log ''
Log '--- done'
exit 0
