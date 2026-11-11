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

    # 10. R18-2: the installed package list belongs to the person, so it is never an authority. A hostile list is refused
    #     before anything moves, and the person's data stays exactly where it was.
    $erin = Join-Path $users 'erin'; New-Item -ItemType Directory -Path $erin | Out-Null
    $r = Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $common + @('-Users', 'erin'))
    $erinApp = & $app 'erin'
    $erinData = Join-Path $erinApp 'BDFData'
    New-Item -ItemType Directory -Path "$erinData\recovery" -Force | Out-Null
    Set-Content "$erinData\settings.json" 'erin settings'
    Set-Content "$erinData\recovery\doc.autosave" 'erin unsaved work'
    $listPath = Join-Path $erinApp 'bdf-package.json'
    $goodList = [IO.File]::ReadAllText($listPath)
    $utf8 = New-Object Text.UTF8Encoding($false)
    function Hostile-List([scriptblock]$Edit) {
        $o = $goodList | ConvertFrom-Json
        & $Edit $o
        [IO.File]::WriteAllText($listPath, ($o | ConvertTo-Json -Depth 6), $utf8)
    }
    function Add-Entry($o, [string]$Path) { $o.files = @($o.files) + [pscustomobject]@{ path = $Path; size = 1; sha256 = ('0' * 64) } }
    $sentinels = { (Get-Content "$erinData\settings.json") -eq 'erin settings' -and (Get-Content "$erinData\recovery\doc.autosave") -eq 'erin unsaved work' }
    $cases = [ordered]@{
        'a native Windows path under BDFData'        = { param($o) Add-Entry $o 'BDFData\settings.json' }
        'a slash path under BDFData'                 = { param($o) Add-Entry $o 'BDFData/settings.json' }
        'BDFData in another letter case'             = { param($o) Add-Entry $o 'bdfdata/recovery/doc.autosave' }
        'BDFData by its 8.3 short name'              = { param($o) Add-Entry $o 'BDFDAT~1/settings.json' }
        'a traversal out of the folder'              = { param($o) Add-Entry $o '../../outside.txt' }
        'a drive path'                               = { param($o) Add-Entry $o 'C:/Windows/win.ini' }
        'an absolute path'                           = { param($o) Add-Entry $o '/Windows/win.ini' }
        'an alternate data stream'                   = { param($o) Add-Entry $o 'bdf.exe:evil' }
        'the update folder'                          = { param($o) Add-Entry $o 'BDFUpdate/x' }
        'the previous-version folder'                = { param($o) Add-Entry $o 'BDFPrevious/x' }
        'the package list itself'                    = { param($o) Add-Entry $o 'bdf-package.json' }
        'the same file twice in another letter case' = { param($o) Add-Entry $o 'BDF.EXE' }
        'no files at all'                            = { param($o) $o.files = @() }
        'a wrong schema'                             = { param($o) $o.schema = 7 }
        'another product'                            = { param($o) $o.product = 'other' }
        'a version that is not a version'            = { param($o) $o.version = '9..x' }
        'an impossible number of files'              = { param($o) $o.files = @(1..2001 | ForEach-Object { [pscustomobject]@{ path = "f$_.txt"; size = 1; sha256 = ('0' * 64) } }) }
    }
    foreach ($name in $cases.Keys) {
        Hostile-List $cases[$name]
        $before = (Get-Content (Join-Path $erinApp 'bdf.exe')) -join ''
        $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'erin'))
        $safe = $r.Code -eq 1 -and $r.Out -match 'cannot be trusted' -and (& $sentinels) -and (-not (Test-Path (Join-Path $erinApp 'BDFPrevious'))) `
            -and ((Get-Content (Join-Path $erinApp 'bdf.exe')) -join '') -eq $before -and -not (Test-Path (Join-Path $erinApp 'BDFUpdate\deploy-stage'))
        Check $safe "a hostile installed list ($name) is refused and BDFData stays where it was"
    }
    # A hostile list that is not even JSON.
    [IO.File]::WriteAllText($listPath, '{ not json', $utf8)
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'erin'))
    Check ($r.Code -eq 1 -and (& $sentinels) -and -not (Test-Path (Join-Path $erinApp 'BDFPrevious'))) 'an installed list that is not JSON is refused and nothing moves'
    # The same list, honest, with a stale file the old version owned: that file is displaced into BDFPrevious, the data is not.
    Set-Content (Join-Path $erinApp 'stale.txt') 'old only'
    Hostile-List { param($o) Add-Entry $o 'stale.txt' }
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'erin'))
    Check ($r.Code -eq 0 -and $r.Out -match 'updated 0.8.1 -> 0.8.2') 'an honest installed list still updates'
    Check ((-not (Test-Path (Join-Path $erinApp 'stale.txt'))) -and (Test-Path (Join-Path $erinApp 'BDFPrevious\stale.txt'))) 'a file only the old version had is kept aside in BDFPrevious'
    Check ((& $sentinels) -and -not (Test-Path (Join-Path $erinApp 'BDFPrevious\BDFData'))) 'BDFData was neither moved nor copied by the update'

    # 11. R18-2: links inside the BDF folder are never followed by anything that changes it.
    $frank = Join-Path $users 'frank'; New-Item -ItemType Directory -Path $frank | Out-Null
    $r = Deploy (@('-PackageZip', $zip1, '-ReleaseFile', $rel1) + $common + @('-Users', 'frank'))
    $frankApp = & $app 'frank'
    $target = Join-Path $Work 'link-target'
    New-Item -ItemType Directory -Path $target | Out-Null
    Set-Content (Join-Path $target 'sentinel.txt') 'must survive'
    function Target-Intact { @(Get-ChildItem $target -Force).Count -eq 1 -and (Get-Content (Join-Path $target 'sentinel.txt')) -eq 'must survive' }
    foreach ($place in 'planted', 'BDFUpdate\deploy-stage', 'BDFUpdate\deploy-previous', 'BDFPrevious', 'BDFUpdate\downloads') {
        $link = Join-Path $frankApp $place
        New-Item -ItemType Directory -Path (Split-Path $link -Parent) -Force | Out-Null
        cmd /c mklink /J "$link" "$target" | Out-Null
        $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'frank'))
        Check ($r.Code -eq 1 -and $r.Out -match 'link or junction' -and (Target-Intact) -and (Get-Content (Join-Path $frankApp 'bdf.exe')) -match 'one') "a junction at $place is refused and its target is untouched"
        cmd /c rmdir "$link" | Out-Null
    }
    # A link inside the person's own BDFData is theirs: the script neither lists nor follows it.
    $dataLink = Join-Path $frankApp 'BDFData\mine'
    New-Item -ItemType Directory -Path (Split-Path $dataLink -Parent) -Force | Out-Null
    cmd /c mklink /J "$dataLink" "$target" | Out-Null
    $r = Deploy (@('-PackageZip', $zip2, '-ReleaseFile', $rel2) + $common + @('-Users', 'frank'))
    Check ($r.Code -eq 0 -and (Target-Intact) -and (Get-Content (Join-Path $frankApp 'bdf.exe')) -match 'two') 'a junction inside BDFData is ignored: the update succeeds and the target is untouched'
    cmd /c rmdir "$dataLink" | Out-Null

    # 12. R18-2: the helpers themselves, loaded from the script, on a tree that holds a link.
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($Deploy, [ref]$null, [ref]$null)
    $wanted = 'Test-IsLink', 'Remove-Tree', 'Assert-Inside', 'Find-Link', 'Initialize-LogDirectory'
    $ReparseAttr = [System.IO.FileAttributes]::ReparsePoint
    $Reserved = @('bdfdata', 'bdfupdate', 'bdfprevious')
    $System32 = Join-Path $env:SystemRoot 'System32'
    foreach ($fn in $ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $wanted -contains $n.Name }, $true)) {
        . ([scriptblock]::Create($fn.Extent.Text))
    }
    $tree = Join-Path $Work 'tree'
    New-Item -ItemType Directory -Path "$tree\a\b" -Force | Out-Null
    Set-Content "$tree\a\b\file.txt" 'x'
    (Get-Item "$tree\a\b\file.txt").Attributes = 'ReadOnly'
    cmd /c mklink /J "$tree\a\link" "$target" | Out-Null
    Remove-Tree $tree
    Check ((-not (Test-Path $tree)) -and (Target-Intact)) 'Remove-Tree deletes a tree with a junction and a read-only file without touching the junction target'
    $appDir = Join-Path $Work 'appdir'
    New-Item -ItemType Directory -Path "$appDir\BDFData", "$appDir\sub" -Force | Out-Null
    $threw = { param($sb) try { & $sb; $false } catch { $true } }
    Check (-not (& $threw { Assert-Inside $appDir (Join-Path $appDir 'sub\x.txt') })) 'Assert-Inside accepts an ordinary file in the folder'
    Check (& $threw { Assert-Inside $appDir (Join-Path $appDir 'BDFData\settings.json') }) 'Assert-Inside refuses BDFData'
    Check (& $threw { Assert-Inside $appDir (Join-Path $appDir 'BDFData\settings.json') -Internal }) 'Assert-Inside refuses BDFData even for internal paths'
    Check (& $threw { Assert-Inside $appDir (Join-Path $appDir 'BDFPrevious\x') }) 'Assert-Inside refuses BDFPrevious for payload paths'
    Check (-not (& $threw { Assert-Inside $appDir (Join-Path $appDir 'BDFPrevious\x') -Internal })) 'Assert-Inside allows BDFPrevious for internal paths'
    Check (& $threw { Assert-Inside $appDir (Join-Path $appDir '..\outside.txt') }) 'Assert-Inside refuses a path that climbs out of the folder'
    Check (& $threw { Assert-Inside $appDir ($appDir + '2\x.txt') }) 'Assert-Inside refuses a sibling folder that shares the name prefix'
    cmd /c mklink /J "$appDir\sub\hop" "$target" | Out-Null
    Check (& $threw { Assert-Inside $appDir (Join-Path $appDir 'sub\hop\evil.txt') }) 'Assert-Inside refuses a path that goes through a link'
    Check ((Find-Link $appDir) -like '*hop') 'Find-Link finds a link inside the folder'
    cmd /c rmdir "$appDir\sub\hop" | Out-Null
    cmd /c mklink /J "$appDir\BDFData\mine" "$target" | Out-Null
    Check ($null -eq (Find-Link $appDir)) 'Find-Link does not look inside BDFData'
    cmd /c rmdir "$appDir\BDFData\mine" | Out-Null
    # The log folder: a link, or a folder owned by an ordinary user, is never written to.
    $savedPd = $env:ProgramData
    try {
        $fakePd = Join-Path $Work 'ProgramData'
        New-Item -ItemType Directory -Path $fakePd | Out-Null
        $env:ProgramData = $fakePd
        $LogDirectory = $null
        cmd /c mklink /J "$fakePd\BDF" "$target" | Out-Null
        Check ($null -eq (Initialize-LogDirectory)) 'a log folder that is a link is not used'
        cmd /c rmdir "$fakePd\BDF" | Out-Null
        New-Item -ItemType Directory -Path "$fakePd\BDF" | Out-Null
        $owner = (Get-Acl "$fakePd\BDF").Owner
        Check (($owner -match '^(NT AUTHORITY\\SYSTEM|BUILTIN\\Administrators)$') -or ($null -eq (Initialize-LogDirectory))) 'a log folder owned by an ordinary user is not used'
    } finally { $env:ProgramData = $savedPd }

    # 13. The person's own task (-AsUser): what Local System starts as the person. It installs with the rights of whoever runs it,
    #     says what happened in the outcome file, verifies the release again, and never asks for an ownership or permission change.
    $gina = Join-Path $users 'gina'; New-Item -ItemType Directory -Path $gina | Out-Null
    $ginaApp = & $app 'gina'
    $outFile = Join-Path $Work 'gina-outcome.txt'
    function AsUser([string]$Zip, [string]$Rel, [string]$Folder = $gina) {
        if (Test-Path $outFile) { Remove-Item $outFile }
        $r = Deploy @('-PackageZip', $Zip, '-ReleaseFile', $Rel, '-PublicKey', $pub, '-Channel', 'dev', '-AsUser', $Folder, '-OutcomeFile', $outFile)
        $text = if (Test-Path $outFile) { Get-Content $outFile -Raw } else { '' }
        [pscustomobject]@{ Code = $r.Code; Outcome = $text }
    }
    $r = AsUser $zip1 $rel1
    Check ($r.Code -eq 0 -and $r.Outcome -match 'RESULT: installed 0.8.1' -and (Get-Content (Join-Path $ginaApp 'bdf.exe')) -match 'one') '-AsUser installs and reports it in the outcome file'
    $r = AsUser $zip1 $rel1
    Check ($r.Code -eq 0 -and $r.Outcome -match 'RESULT: current') '-AsUser on a current copy reports current and exits 0'
    $hold = [IO.File]::Open((Join-Path $ginaApp 'bdf-cli.exe'), 'Open', 'Read', 'Read')
    $r = AsUser $zip2 $rel2
    $hold.Dispose()
    Check ($r.Code -eq 4 -and $r.Outcome -match 'RESULT: in-use' -and (Get-Content (Join-Path $ginaApp 'bdf.exe')) -match 'one') '-AsUser with BDF running exits 4 and changes nothing'
    $r = AsUser $zip2 $rel2
    Check ($r.Code -eq 0 -and $r.Outcome -match 'RESULT: updated 0.8.1 -> 0.8.2' -and (Get-Content (Join-Path $ginaApp 'bdf.exe')) -match 'two') '-AsUser updates'
    New-Item -ItemType Directory -Path "$ginaApp\BDFUpdate" -Force | Out-Null
    Set-Content "$ginaApp\BDFUpdate\deploy-pending.json" '{}'
    $r = AsUser $zip2 $rel2
    Check ($r.Code -eq 0 -and $r.Outcome -match 'RESULT: updated 0.8.2 -> 0.8.2' -and -not (Test-Path "$ginaApp\BDFUpdate\deploy-pending.json")) 'an unfinished-run marker makes a current-looking copy be finished, and the marker goes away'
    $bogus = Join-Path $Work 'bogus.zip'; Set-Content $bogus 'this is not a zip'
    $r = AsUser $bogus $rel2
    Check ($r.Code -eq 0 -and $r.Outcome -match 'RESULT: current') 'a person already current costs only the signature check: the package is not even read'
    $ivan = Join-Path $users 'ivan'; New-Item -ItemType Directory -Path $ivan | Out-Null
    $r = AsUser $bad $rel2 $ivan
    Check ($r.Code -eq 1 -and $r.Outcome -match 'STOP' -and -not (Test-Path (Join-Path $ivan 'AppData'))) '-AsUser verifies the release itself for a person who needs it: a tampered package is refused (exit 1) and nothing is installed'
    # 14. What the sign-in task runs (-AsCurrentUser): the person running it, only if they are among -Users, and a text result for Local System.
    $zed = Join-Path $users 'zed'; New-Item -ItemType Directory -Path $zed | Out-Null
    $resultsDir = Join-Path $Work 'results'; New-Item -ItemType Directory -Path $resultsDir | Out-Null
    function AsCurrent([string]$Zip, [string]$Rel, [string[]]$Extra = @()) {
        $savedProfile = $env:USERPROFILE; $savedName = $env:USERNAME
        try {
            $env:USERPROFILE = $zed; $env:USERNAME = 'zed'     # the child process sees a fake person; nothing real is touched
            Deploy (@('-PackageZip', $Zip, '-ReleaseFile', $Rel, '-PublicKey', $pub, '-Channel', 'dev', '-AsCurrentUser', '-ResultsDir', $resultsDir) + $Extra)
        } finally { $env:USERPROFILE = $savedProfile; $env:USERNAME = $savedName }
    }
    $r = AsCurrent $zip1 $rel1 @('-Users', 'someone-else')
    Check ($r.Code -eq 0 -and $r.Out -match 'not among -Users' -and -not (Test-Path (Join-Path $zed 'AppData\Local\Programs\BDF'))) ("-AsCurrentUser does nothing for a person who is not among -Users (exit $($r.Code): $(($r.Out -replace '\s+', ' ').Trim())")
    $r = AsCurrent $zip1 $rel1 @('-Users', 'zed')
    Check ($r.Code -eq 0 -and (Get-Content (Join-Path $zed 'AppData\Local\Programs\BDF\bdf.exe')) -match 'one') '-AsCurrentUser installs for the person running it'
    Check ((Get-Content (Join-Path $resultsDir 'zed.txt') -Raw) -match 'RESULT: installed 0.8.1') 'and leaves the result in a file named for them'
    $r = AsCurrent $zip1 $rel1
    Check ($r.Code -eq 0 -and (Get-Content (Join-Path $resultsDir 'zed.txt') -Raw) -match 'RESULT: current') 'at the next sign-in it finds nothing to do and says so'
    $r = AsCurrent $zip2 $rel2
    Check ($r.Code -eq 0 -and (Get-Content (Join-Path $zed 'AppData\Local\Programs\BDF\bdf.exe')) -match 'two' -and (Get-Content (Join-Path $resultsDir 'zed.txt') -Raw) -match 'RESULT: updated 0.8.1 -> 0.8.2') 'when a newer release is published, the next sign-in updates it'
    $hank = Join-Path $users 'hank'; New-Item -ItemType Directory -Path "$hank\AppData\Local\Programs\BDF" -Force | Out-Null
    Set-Content "$hank\AppData\Local\Programs\BDF\precious.txt" 'not BDF'
    $r = AsUser $zip2 $rel2 $hank
    Check ($r.Code -eq 1 -and $r.Outcome -match 'RESULT: FAILED' -and (Test-Path "$hank\AppData\Local\Programs\BDF\precious.txt")) '-AsUser on a folder that is not a managed copy fails and leaves it alone'
} finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Output "$failures check(s) failed"; exit 1 }
Write-Output 'all deployment checks passed'
# Explicit: a native command above may have left $LASTEXITCODE non-zero, and a CI shell would report that instead.
exit 0
