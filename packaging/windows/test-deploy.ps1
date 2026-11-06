<#
.SYNOPSIS
  Tests deploy-bdf.ps1 against synthetic packages and fake profile folders. Touches nothing outside a temp folder.
.DESCRIPTION
  Builds two small signed packages (0.8.1 and 0.8.2, fake program files) with `cargo xtask`, then runs the
  deployment script as the current user with -UsersRoot (so there is no ownership change). Run from the
  repository root: powershell -File packaging\windows\test-deploy.ps1
#>
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Deploy = Join-Path $PSScriptRoot 'deploy-bdf.ps1'
$Work = Join-Path ([IO.Path]::GetTempPath()) ("bdf-deploy-test-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $Work | Out-Null
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Output "ok   $what" } else { Write-Output "FAIL $what"; $script:failures++ }
}
function Xtask { & cargo run -q -p xtask -- @args; if ($LASTEXITCODE) { throw "xtask $($args[0]) failed" } }
function Deploy([string[]]$Extra) {
    $out = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Deploy @Extra 2>&1 | Out-String
    [pscustomobject]@{ Code = $LASTEXITCODE; Out = $out }
}

try {
    Push-Location $Root
    $keyFile = Join-Path $Work 'key.txt'
    $keygen = Xtask update-keygen --out $keyFile | Out-String
    $pub = ($keygen -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '' | ForEach-Object { $_.Trim() }
    $bin = Join-Path $Work 'bin'
    New-Item -ItemType Directory -Path $bin | Out-Null
    function Make-Release([string]$Version, [string]$Marker) {
        foreach ($n in 'bdf.exe', 'bdf-cli.exe', 'bdf-worker.exe', 'bdf-updater.exe') { Set-Content -Path (Join-Path $bin $n) -Value "$n $Version $Marker" -Encoding ascii }
        $out = Join-Path $Work "release-$Version"
        Xtask portable-package --bin-dir $bin --version $Version --out $out | Out-Null
        Xtask update-sign --zip (Join-Path $out "bdf-$Version-windows-x64-portable.zip") --version $Version --key $keyFile --out $out --channel dev --url-base "http://127.0.0.1:1/v$Version" | Out-Null
        $out
    }
    $r1 = Make-Release '0.8.1' 'one'
    $r2 = Make-Release '0.8.2' 'two'
    $zip1 = Join-Path $r1 'bdf-0.8.1-windows-x64-portable.zip'
    $zip2 = Join-Path $r2 'bdf-0.8.2-windows-x64-portable.zip'
    $rel1 = Join-Path $r1 'bdf-update.json'
    $rel2 = Join-Path $r2 'bdf-update.json'
    Pop-Location

    $users = Join-Path $Work 'Users'
    foreach ($u in 'alice', 'bob', 'Default') { New-Item -ItemType Directory -Path (Join-Path $users $u) | Out-Null }
    $common = @('-PublicKey', $pub, '-Channel', 'dev', '-UsersRoot', $users)
    $app = { param($u) Join-Path $users "$u\AppData\Local\Programs\BDF" }

    # 1. first deployment, for real users only (Default is not touched without the switch)
    $r = Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $common + @('-Users', 'alice,bob'))
    Check ($r.Code -eq 0) 'first deployment succeeds'
    Check ((Get-Content (Join-Path (& $app 'alice') 'bdf.exe')) -match 'one') 'alice has 0.8.1'
    Check (Test-Path (Join-Path (& $app 'bob') 'portable.txt')) 'bob has portable.txt (portable mode)'
    Check (-not (Test-Path (& $app 'Default'))) 'the Default profile was not touched without -IncludeDefaultProfile'
    Check (-not (Test-Path (Join-Path (& $app 'alice') 'BDFData'))) 'the package ships no data folder'

    # 2. rerun is a no-op
    $stamp = (Get-Item (Join-Path (& $app 'alice') 'bdf.exe')).LastWriteTimeUtc
    $r = Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $common + @('-Users', 'alice,bob'))
    Check ($r.Code -eq 0 -and $r.Out -match 'already has 0.8.1') 'a rerun changes nothing'
    Check ((Get-Item (Join-Path (& $app 'alice') 'bdf.exe')).LastWriteTimeUtc -eq $stamp) 'the program file was not rewritten'

    # 3. update keeps user data and the user's own files, keeps the previous version
    $data = Join-Path (& $app 'alice') 'BDFData'
    New-Item -ItemType Directory -Path "$data\recovery" -Force | Out-Null
    Set-Content "$data\settings.json" '{"theme_choice":"dark"}'
    Set-Content "$data\recovery\doc.autosave" 'unsaved work'
    Set-Content (Join-Path (& $app 'alice') 'notes.txt') 'my notes'
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'alice'))
    Check ($r.Code -eq 0 -and $r.Out -match 'updated 0.8.1 -> 0.8.2') 'update 0.8.1 to 0.8.2'
    Check ((Get-Content (Join-Path (& $app 'alice') 'bdf.exe')) -match 'two') 'alice now has 0.8.2'
    Check ((Get-Content "$data\settings.json") -eq '{"theme_choice":"dark"}' -and (Get-Content "$data\recovery\doc.autosave") -eq 'unsaved work') 'BDFData is untouched'
    Check ((Get-Content (Join-Path (& $app 'alice') 'notes.txt')) -eq 'my notes') "the user's own file is untouched"
    Check ((Get-Content (Join-Path (& $app 'alice') 'BDFPrevious\bdf.exe')) -match 'one') 'the previous version is kept'
    Check ((Get-Content (Join-Path (& $app 'bob') 'bdf.exe')) -match 'one') 'bob (not in -Users) was not updated'

    # 4. a running copy is never replaced; the next run does it
    $bobExe = Join-Path (& $app 'bob') 'bdf-cli.exe'
    $hold = [IO.File]::Open($bobExe, 'Open', 'Read', 'Read')
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'bob'))
    $hold.Dispose()
    Check ($r.Code -eq 0 -and $r.Out -match 'running from its folder') 'a running copy is skipped, not failed'
    Check ((Get-Content (Join-Path (& $app 'bob') 'bdf.exe')) -match 'one') 'the running copy is unchanged'
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'bob'))
    Check ((Get-Content (Join-Path (& $app 'bob') 'bdf.exe')) -match 'two') 'the next run updates it'

    # 5. older package: nothing happens
    $r = Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $common + @('-Users', 'alice'))
    Check ((Get-Content (Join-Path (& $app 'alice') 'bdf.exe')) -match 'two') 'an older release is never installed over a newer copy'

    # 6. default profile, for users created later
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'nobody', '-IncludeDefaultProfile'))
    Check (Test-Path (Join-Path (& $app 'Default') 'bdf.exe')) '-IncludeDefaultProfile installs for users created later'

    # 7. tampered package, tampered release, wrong key, wrong channel
    $bad = Join-Path $Work 'tampered.zip'
    Copy-Item $zip2 $bad
    $bytes = [IO.File]::ReadAllBytes($bad); $bytes[[int]($bytes.Length / 2)] = $bytes[[int]($bytes.Length / 2)] -bxor 0x55; [IO.File]::WriteAllBytes($bad, $bytes)
    $fresh = Join-Path $users 'carol'; New-Item -ItemType Directory -Path $fresh | Out-Null
    $r = Deploy (@('-PackageZip', $bad, '-ReleaseFile', $rel2) + $common + @('-Users', 'carol'))
    Check ($r.Code -eq 1 -and -not (Test-Path (Join-Path $fresh 'AppData'))) 'a tampered package installs nothing'
    $badRel = Join-Path $Work 'bad-release.json'
    (Get-Content $rel2 -Raw) -replace '"signature": "', '"signature": "0' | Set-Content $badRel -Encoding utf8
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $badRel) + $common + @('-Users', 'carol'))
    Check ($r.Code -eq 1 -and -not (Test-Path (Join-Path $fresh 'AppData'))) 'a tampered release file installs nothing'
    $otherKey = Join-Path $Work 'other.txt'
    Push-Location $Root; $k2 = Xtask update-keygen --out $otherKey | Out-String; Pop-Location
    $pub2 = ($k2 -split "`n" | Where-Object { $_ -like 'public key*' }) -replace '.*: ', '' | ForEach-Object { $_.Trim() }
    $r = Deploy @('-PackageZip', $zip2, '-ReleaseFile', $rel2, '-PublicKey', $pub2, '-Channel', 'dev', '-UsersRoot', $users, '-Users', 'carol')
    Check ($r.Code -eq 1 -and $r.Out -match 'not trusted' -and -not (Test-Path (Join-Path $fresh 'AppData'))) 'a release signed by another key is refused'
    $r = Deploy @('-PackageZip', $zip2, '-ReleaseFile', $rel2, '-PublicKey', $pub, '-UsersRoot', $users, '-Users', 'carol')
    Check ($r.Code -eq 1 -and $r.Out -match 'channel') 'a dev release is refused by a stable run'

    # 8. a folder that is not a managed copy, and a planted junction, are left alone
    $odd = Join-Path (Join-Path $users 'carol') 'AppData\Local\Programs\BDF'
    New-Item -ItemType Directory -Path $odd -Force | Out-Null
    Set-Content (Join-Path $odd 'precious.txt') 'not BDF'
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'carol'))
    Check ($r.Code -eq 1 -and (Test-Path (Join-Path $odd 'precious.txt')) -and -not (Test-Path (Join-Path $odd 'bdf.exe'))) 'an unmanaged folder is left alone'
    Remove-Item $odd -Recurse -Force
    $elsewhere = Join-Path $Work 'elsewhere'; New-Item -ItemType Directory -Path $elsewhere | Out-Null
    New-Item -ItemType Directory -Path (Split-Path $odd -Parent) -Force | Out-Null
    cmd /c mklink /J "$odd" "$elsewhere" | Out-Null
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'carol'))
    Check ($r.Code -eq 1 -and @(Get-ChildItem $elsewhere).Count -eq 0) 'a junction in the way is not written through'
    cmd /c rmdir "$odd" | Out-Null

    # 9. -WhatIf changes nothing
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'dave', '-WhatIf'))
    New-Item -ItemType Directory -Path (Join-Path $users 'dave') -Force | Out-Null
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'dave', '-WhatIf'))
    Check ($r.Out -match 'would install' -and -not (Test-Path (Join-Path $users 'dave\AppData'))) '-WhatIf reports and changes nothing'
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all deployment checks passed'
