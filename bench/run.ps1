# Compares Bach with Inno Setup, NSIS and WiX on one real app (see
# make-payload.ps1): build time and size, install, update and removal time,
# and what happens when things go wrong (a locked file during an update, a
# damaged installer, an installer that is killed half way, user files in the
# install folder). Every installer installs per user, without administrator
# rights, so no prompt can appear and every tool gets the same job.
#
#   powershell -ExecutionPolicy Bypass -File bench\run.ps1 -Work D:\bench-work -Tools bach,inno
#
# It writes results\<tool>.json next to this script and installs only into
# <Work>\install\<tool>. It removes the registry entries it made (the ones
# named BenchApp) when it is done. Run it on a quiet computer: timings are
# wall-clock, and a virus scanner that scans every new file changes them.

param(
    [Parameter(Mandatory)][string]$Work,
    [string[]]$Tools = @('bach', 'inno', 'nsis', 'wix'),
    [int]$Repeats = 3,
    [string]$BachBin = '',
    [string]$Results = '',
    [int]$StepTimeout = 300,
    [switch]$SkipBuilds
)
$ErrorActionPreference = 'Stop'
# $PSScriptRoot is empty in a param default under Windows PowerShell 5.1, so these are set here.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $BachBin) { $BachBin = Join-Path $here '..\target\release\bach.exe' }
if (-not $Results) { $Results = Join-Path $here 'results' }
$BachBin = [IO.Path]::GetFullPath($BachBin)
$Work = (Resolve-Path $Work).Path
$Results = [IO.Path]::GetFullPath($Results)
New-Item -ItemType Directory -Force $Results, (Join-Path $Work 'out'), (Join-Path $Work 'cfg'), (Join-Path $Work 'install') | Out-Null
$ISCC = Join-Path $env:USERPROFILE 'scoop\apps\inno-setup\current\ISCC.exe'
$MAKENSIS = Join-Path $env:USERPROFILE 'scoop\apps\nsis\current\makensis.exe'
$DOTNET_ROOT = Join-Path $env:USERPROFILE 'scoop\apps\dotnet-sdk\current'
$WIX = Join-Path $env:USERPROFILE 'scoop\apps\wixtoolset\current\wix.exe'
$ENTRY_NAME = 'BenchApp'
$UNINSTALL_KEYS = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'

# ---------------------------------------------------------------- helpers

function Run-Timed([string]$File, [string]$Arguments, [int]$TimeoutSec = $StepTimeout) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $env:DOTNET_ROOT = $DOTNET_ROOT
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    $peak = 0L
    $timedOut = $false
    while (-not $p.WaitForExit(200)) {
        try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peak) { $peak = $p.PeakWorkingSet64 } } catch { }
        if ($sw.Elapsed.TotalSeconds -gt $TimeoutSec) {
            $timedOut = $true
            try { & taskkill.exe /F /T /PID $p.Id 2>&1 | Out-Null } catch { }
            break
        }
    }
    $sw.Stop()
    try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peak) { $peak = $p.PeakWorkingSet64 } } catch { }
    [pscustomobject]@{
        Seconds  = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        ExitCode = $(if ($timedOut) { $null } else { $p.ExitCode })
        TimedOut = $timedOut
        PeakMB   = [math]::Round($peak / 1MB, 0)
    }
}

function Find-Entries {
    foreach ($root in $UNINSTALL_KEYS) {
        Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {
            $name = (Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue).DisplayName
            if ($name -eq $ENTRY_NAME) { $_ }
        }
    }
}

function Dir-Files([string]$Dir) {
    if (Test-Path $Dir) { @(Get-ChildItem $Dir -Recurse -File -Force -ErrorAction SilentlyContinue) } else { @() }
}

# What an installer is allowed to add next to the app's files.
$ownFiles = '^(uninstall\.exe|unins\d+\.(exe|dat|msg)|\.bach.*)$'

function Tree([string]$Dir) {
    $map = @{}
    $root = $Dir.TrimEnd('\') + '\'
    foreach ($f in (Dir-Files $Dir)) {
        $rel = $f.FullName.Substring($root.Length)
        if ($rel -match $ownFiles) { continue }
        $map[$rel.ToLowerInvariant()] = $f.Length
    }
    $map
}

# How an install folder differs from a payload: missing and extra files, files
# of the wrong size, and (on a fixed sample of 400 files) of the wrong content.
function Compare-Tree([string]$Payload, [string]$Installed) {
    $want = Tree $Payload
    $have = Tree $Installed
    $missing = 0; $wrong = 0
    foreach ($k in $want.Keys) {
        if (-not $have.ContainsKey($k)) { $missing++ } elseif ($have[$k] -ne $want[$k]) { $wrong++ }
    }
    $extra = 0
    foreach ($k in $have.Keys) { if (-not $want.ContainsKey($k)) { $extra++ } }
    $sample = @($want.Keys | Sort-Object | Get-Random -Count ([math]::Min(400, $want.Count)) -SetSeed 11)
    $badHash = 0
    foreach ($k in $sample) {
        $a = Join-Path $Payload $k; $b = Join-Path $Installed $k
        if (-not (Test-Path -LiteralPath $b)) { continue }
        if ((Get-FileHash -LiteralPath $a -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $b -Algorithm SHA256).Hash) { $badHash++ }
    }
    [pscustomobject]@{ Expected = $want.Count; Found = $have.Count; Missing = $missing; Extra = $extra; WrongSize = $wrong; WrongContent = $badHash; Identical = ($missing -eq 0 -and $extra -eq 0 -and $wrong -eq 0 -and $badHash -eq 0) }
}

function Force-Clean([string]$Dir) {
    foreach ($e in @(Find-Entries)) { Remove-Item -LiteralPath $e.PSPath -Recurse -Force -ErrorAction SilentlyContinue }
    if (Test-Path $Dir) {
        for ($i = 0; $i -lt 5; $i++) {
            Remove-Item -LiteralPath $Dir -Recurse -Force -ErrorAction SilentlyContinue
            if (-not (Test-Path $Dir)) { break }
            Start-Sleep -Seconds 2
        }
    }
}

# Seconds until the app is gone: its folder and its entry in Settings > Apps.
function Wait-Removed([string]$Dir, [int]$TimeoutSec) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
        $files = @(Dir-Files $Dir).Count
        if ($files -eq 0 -and @(Find-Entries).Count -eq 0) { return [math]::Round($sw.Elapsed.TotalSeconds, 1) }
        Start-Sleep -Milliseconds 300
    }
    $null
}

# The first-level folders and keys that exist now, to see what an app leaves behind.
function Snapshot {
    $set = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($root in $env:LOCALAPPDATA, $env:APPDATA, [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop'), "$env:LOCALAPPDATA\Programs") {
        Get-ChildItem $root -Force -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add($_.FullName) }
    }
    Get-ChildItem 'HKCU:\Software' -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add($_.Name) }
    Get-ChildItem $UNINSTALL_KEYS[0] -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add($_.Name) }
    $set
}

# ---------------------------------------------------------------- the tools

function Write-File([string]$Path, [string]$Text) { [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false)) }

function Define-Tool([string]$Tool) {
    switch ($Tool) {
        'bach' {
            @{
                Profiles = @('default', 'fast')
                Build = {
                    param($Profile, $Version, $Payload, $Out)
                    $cfg = Join-Path $Work "cfg\bach-$Profile-$Version.toml"
                    Write-File $cfg @"
[app]
name = "BenchApp"
id = "BenchApp"
version = "$Version"
publisher = "Bench Inc."
executable = "BenchApp.exe"

[files]
source = '$Payload'
allow_secrets = ["**/*.key", "**/*.pem", "**/.env*", "**/*.p12", "**/*.pfx"]

[installer]
output = '$Out'
folder = "BenchApp"
desktop_shortcut = false
start_menu_shortcut = "never"
open_button = false
"@
                    $extra = $(if ($Profile -eq 'fast') { ' --fast' } else { '' })
                    Run-Timed $BachBin "build `"$cfg`"$extra" 3600
                }
                Install = { param($Setup, $Dir) @{ File = $Setup; Args = "--silent --dir `"$Dir`"" } }
                Uninstall = { param($Setup, $Dir) @{ File = (Join-Path $Dir 'uninstall.exe'); Args = '--silent' } }
            }
        }
        'inno' {
            @{
                Profiles = @('default', 'max')
                Build = {
                    param($Profile, $Version, $Payload, $Out)
                    $iss = Join-Path $Work "cfg\inno-$Profile-$Version.iss"
                    $comp = $(if ($Profile -eq 'max') { "Compression=lzma2/ultra64`nSolidCompression=yes" } else { '' })
                    Write-File $iss @"
[Setup]
AppId=BenchApp-Bench
AppName=BenchApp
AppVersion=$Version
AppPublisher=Bench Inc.
DefaultDirName={localappdata}\Programs\BenchApp
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
DisableDirPage=yes
OutputDir=$(Split-Path $Out)
OutputBaseFilename=$([IO.Path]::GetFileNameWithoutExtension($Out))
$comp

[Files]
Source: "$Payload\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion
"@
                    Run-Timed $ISCC "/Q `"$iss`"" 3600
                }
                Install = { param($Setup, $Dir) @{ File = $Setup; Args = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /DIR=`"$Dir`"" } }
                Uninstall = { param($Setup, $Dir) @{ File = (Join-Path $Dir 'unins000.exe'); Args = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' } }
            }
        }
        'nsis' {
            @{
                Profiles = @('default', 'max')
                Build = {
                    param($Profile, $Version, $Payload, $Out)
                    $nsi = Join-Path $Work "cfg\nsis-$Profile-$Version.nsi"
                    $comp = $(if ($Profile -eq 'max') { 'SetCompressor /SOLID lzma' } else { '; the default compressor: zlib, not solid' })
                    Write-File $nsi @"
Unicode true
$comp
Name "BenchApp"
OutFile "$Out"
RequestExecutionLevel user
InstallDir "`$LOCALAPPDATA\Programs\BenchApp"
SilentInstall silent
SilentUnInstall silent
Section
  SetOutPath "`$INSTDIR"
  File /r "$Payload\*.*"
  WriteUninstaller "`$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp" "DisplayName" "BenchApp"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp" "DisplayVersion" "$Version"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp" "Publisher" "Bench Inc."
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp" "InstallLocation" "`$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp" "UninstallString" "`$INSTDIR\uninstall.exe"
SectionEnd
Section "Uninstall"
  RMDir /r "`$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\BenchApp"
SectionEnd
"@
                    Run-Timed $MAKENSIS "/V1 `"$nsi`"" 3600
                }
                Install = { param($Setup, $Dir) @{ File = $Setup; Args = "/S /D=$Dir" } }
                Uninstall = { param($Setup, $Dir) @{ File = (Join-Path $Dir 'uninstall.exe'); Args = '/S' } }
            }
        }
        'wix' {
            @{
                Profiles = @('default', 'high')
                Build = {
                    param($Profile, $Version, $Payload, $Out)
                    $wxs = Join-Path $Work "cfg\wix-$Profile-$Version.wxs"
                    $level = $(if ($Profile -eq 'high') { ' CompressionLevel="high"' } else { '' })
                    Write-File $wxs @"
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="BenchApp" Manufacturer="Bench Inc." Version="$Version" UpgradeCode="6B1E0F3A-52C1-4D6B-9D7B-3F2E1A9C4D10" Scope="perUser" InstallerVersion="500">
    <MajorUpgrade DowngradeErrorMessage="A newer version is installed." />
    <MediaTemplate EmbedCab="yes"$level />
    <StandardDirectory Id="LocalAppDataFolder">
      <Directory Id="ProgramsDir" Name="Programs">
        <Directory Id="INSTALLFOLDER" Name="BenchApp" />
      </Directory>
    </StandardDirectory>
    <Feature Id="Main">
      <ComponentGroupRef Id="AppFiles" />
    </Feature>
  </Package>
  <Fragment>
    <ComponentGroup Id="AppFiles" Directory="INSTALLFOLDER">
      <Files Include="$Payload\**" />
    </ComponentGroup>
  </Fragment>
</Wix>
"@
                    Run-Timed $WIX "build `"$wxs`" -o `"$Out`" -arch x64 -sice ICE38 -sice ICE64 -sice ICE91 -sice ICE03" 3600
                }
                Install = { param($Setup, $Dir) @{ File = "$env:SystemRoot\System32\msiexec.exe"; Args = "/i `"$Setup`" /qn INSTALLFOLDER=`"$Dir`" /l*v `"$Work\cfg\msi-install.log`"" } }
                Uninstall = { param($Setup, $Dir) @{ File = "$env:SystemRoot\System32\msiexec.exe"; Args = "/x `"$Setup`" /qn /l*v `"$Work\cfg\msi-uninstall.log`"" } }
            }
        }
    }
}

# ---------------------------------------------------------------- one tool

function Test-Tool([string]$Tool) {
    $def = Define-Tool $Tool
    $dir = Join-Path $Work "install\$Tool"
    $ext = $(if ($Tool -eq 'wix') { 'msi' } else { 'exe' })
    $r = [ordered]@{ tool = $Tool; started = (Get-Date).ToString('s'); notes = @() }
    $setup = @{}
    foreach ($p in $def.Profiles) { $setup[$p] = Join-Path $Work "out\$Tool-$p-1.0.0.$ext" }
    $maxProfile = $def.Profiles[0]
    $installProfile = $def.Profiles[0]   # what the install tests use: the tool's default build
    $max = $def.Profiles[$def.Profiles.Count - 1]

    Write-Host "== ${Tool}: builds"
    $r.build = [ordered]@{}
    foreach ($p in $def.Profiles) {
        $out = $setup[$p]
        if (-not $SkipBuilds -or -not (Test-Path $out)) {
            if (Test-Path $out) { Remove-Item $out -Force }
            $res = & $def.Build $p '1.0.0' (Join-Path $Work 'payload-1.0.0') $out
        } else { $res = [pscustomobject]@{ Seconds = $null; ExitCode = 0; PeakMB = $null } }
        $size = $(if (Test-Path $out) { [math]::Round((Get-Item $out).Length / 1MB, 1) } else { $null })
        $r.build[$p] = [ordered]@{ seconds = $res.Seconds; exit = $res.ExitCode; peak_mb = $res.PeakMB; size_mb = $size }
        Write-Host ("   {0,-8} {1,7}s  {2} MB  (exit {3}, peak {4} MB)" -f $p, $res.Seconds, $size, $res.ExitCode, $res.PeakMB)
    }
    # Version 1.1.0 of the build the install tests use, and the tiny app.
    $v2 = Join-Path $Work "out\$Tool-$installProfile-1.1.0.$ext"
    if (-not $SkipBuilds -or -not (Test-Path $v2)) {
        if (Test-Path $v2) { Remove-Item $v2 -Force }
        $res = & $def.Build $installProfile '1.1.0' (Join-Path $Work 'payload-1.1.0') $v2
        $r.build['v1_1_0'] = [ordered]@{ seconds = $res.Seconds; exit = $res.ExitCode; peak_mb = $res.PeakMB; size_mb = [math]::Round((Get-Item $v2).Length / 1MB, 1) }
    }
    $tiny = Join-Path $Work "out\$Tool-tiny.$ext"
    if (Test-Path $tiny) { Remove-Item $tiny -Force }
    $res = & $def.Build $installProfile '1.0.0' (Join-Path $Work 'payload-tiny') $tiny
    $payloadTiny = (Get-Item (Join-Path $Work 'payload-tiny\BenchApp.exe')).Length
    $r.tiny = [ordered]@{ installer_kb = [math]::Round((Get-Item $tiny).Length / 1KB, 0); app_kb = [math]::Round($payloadTiny / 1KB, 0); overhead_kb = [math]::Round(((Get-Item $tiny).Length - $payloadTiny) / 1KB, 0) }

    $s1 = $setup[$installProfile]
    $payload1 = Join-Path $Work 'payload-1.0.0'
    $payload2 = Join-Path $Work 'payload-1.1.0'
    Force-Clean $dir
    $before = Snapshot

    # A. install, check, remove: Repeats times
    Write-Host "== ${Tool}: install and remove x$Repeats"
    $r.install = @(); $r.uninstall = @(); $r.verify = $null; $r.after_remove = $null
    for ($i = 1; $i -le $Repeats; $i++) {
        $ins = & $def.Install $s1 $dir
        $t = Run-Timed $ins.File $ins.Args $StepTimeout
        $r.install += [ordered]@{ seconds = $t.Seconds; exit = $t.ExitCode; timed_out = $t.TimedOut }
        if ($i -eq 1) { $r.verify = Compare-Tree $payload1 $dir; $r.entry_written = (@(Find-Entries).Count -gt 0) }
        $un = & $def.Uninstall $s1 $dir
        $t = Run-Timed $un.File $un.Args $StepTimeout
        $gone = Wait-Removed $dir $StepTimeout
        $r.uninstall += [ordered]@{ seconds = $(if ($null -ne $gone) { [math]::Round($t.Seconds + 0, 1) } else { $null }); until_gone_seconds = $gone; exit = $t.ExitCode }
        Write-Host ("   run {0}: install {1}s, remove {2}s (until gone {3}s), identical={4}" -f $i, $r.install[-1].seconds, $r.uninstall[-1].seconds, $gone, $(if ($i -eq 1) { $r.verify.Identical } else { '-' }))
        Force-Clean $dir
    }
    $after = Snapshot
    $left = @($after | Where-Object { -not $before.Contains($_) })
    $r.leftovers_after_remove = $left

    # B. update 1.0.0 -> 1.1.0
    Write-Host "== ${Tool}: update"
    $ins = & $def.Install $s1 $dir; [void](Run-Timed $ins.File $ins.Args $StepTimeout)
    $ins2 = & $def.Install $v2 $dir
    $t = Run-Timed $ins2.File $ins2.Args $StepTimeout
    $cmp = Compare-Tree $payload2 $dir
    $r.update = [ordered]@{ seconds = $t.Seconds; exit = $t.ExitCode; timed_out = $t.TimedOut; identical_to_new = $cmp.Identical; stale_files = $cmp.Extra; missing = $cmp.Missing; wrong_size = $cmp.WrongSize; wrong_content = $cmp.WrongContent }
    Write-Host ("   {0}s exit {1}, identical to 1.1.0: {2}, stale old files: {3}" -f $t.Seconds, $t.ExitCode, $cmp.Identical, $cmp.Extra)
    $un = & $def.Uninstall $v2 $dir; [void](Run-Timed $un.File $un.Args $StepTimeout); [void](Wait-Removed $dir $StepTimeout)
    Force-Clean $dir

    # C. update while a file the update changes is locked (a running app)
    Write-Host "== ${Tool}: locked file during update"
    $ins = & $def.Install $s1 $dir; [void](Run-Timed $ins.File $ins.Args $StepTimeout)
    $lockedRel = 'resources\app\main.js'
    $locked = Join-Path $dir $lockedRel
    $lock = $null
    try {
        $lock = [IO.File]::Open($locked, 'Open', 'Read', 'None')
        $ins2 = & $def.Install $v2 $dir
        $t = Run-Timed $ins2.File $ins2.Args 150
    } finally { if ($lock) { $lock.Close() } }
    $want1 = Tree $payload1; $want2 = Tree $payload2; $have = Tree $dir
    $nowV1 = 0; $nowV2 = 0; $other = 0
    foreach ($k in $want2.Keys) {
        if ($want1.ContainsKey($k) -and $want1[$k] -ne $want2[$k]) {
            if (-not $have.ContainsKey($k)) { $other++ } elseif ($have[$k] -eq $want1[$k]) { $nowV1++ } elseif ($have[$k] -eq $want2[$k]) { $nowV2++ } else { $other++ }
        }
    }
    $newPresent = @($have.Keys | Where-Object { $_ -like 'resources\app\new-in-1.1.0\*' }).Count
    $state = $(if ($nowV2 -eq 0 -and $newPresent -eq 0) { 'old version intact' } elseif ($nowV1 -eq 0 -and $other -eq 0) { 'new version complete' } else { 'mixed: some files new, some old' })
    $r.locked_update = [ordered]@{ seconds = $t.Seconds; exit = $t.ExitCode; timed_out = $t.TimedOut; changed_files_still_old = $nowV1; changed_files_new = $nowV2; new_files_present = $newPresent; state = $state }
    Write-Host ("   exit {0}{1}: {2} (old {3}, new {4}, new files {5})" -f $t.ExitCode, $(if ($t.TimedOut) { ' (hung, killed)' } else { '' }), $state, $nowV1, $nowV2, $newPresent)
    Start-Sleep -Seconds 2
    $un = & $def.Uninstall $s1 $dir; [void](Run-Timed $un.File $un.Args 120); [void](Wait-Removed $dir 60)
    Force-Clean $dir

    # D. a damaged installer: one byte flipped in the middle
    Write-Host "== ${Tool}: damaged installer"
    $damaged = Join-Path $Work "out\$Tool-damaged.$ext"
    Copy-Item $s1 $damaged -Force
    $fs = [IO.File]::Open($damaged, 'Open', 'ReadWrite')
    $fs.Position = [long]($fs.Length * 0.6)
    $byte = $fs.ReadByte()
    $fs.Position = [long]($fs.Length * 0.6)
    $fs.WriteByte([byte]($byte -bxor 0xFF))
    $fs.Close()
    $ins = & $def.Install $damaged $dir
    $t = Run-Timed $ins.File $ins.Args 150
    $filesNow = @(Dir-Files $dir).Count
    $entry = (@(Find-Entries).Count -gt 0)
    $r.damaged = [ordered]@{ seconds = $t.Seconds; exit = $t.ExitCode; timed_out = $t.TimedOut; files_written = $filesNow; entry_written = $entry; refused = ($filesNow -eq 0 -and -not $entry -and $t.ExitCode -ne 0) }
    Write-Host ("   exit {0}, files written {1}, Settings entry {2}" -f $t.ExitCode, $filesNow, $entry)
    if ($entry) { $un = & $def.Uninstall $damaged $dir; [void](Run-Timed $un.File $un.Args 120); [void](Wait-Removed $dir 60) }
    Force-Clean $dir; Remove-Item $damaged -Force -ErrorAction SilentlyContinue

    # E. an installer that is killed half way, then run again
    if ($Tool -ne 'wix') {
        Write-Host "== ${Tool}: killed half way"
        $ins = & $def.Install $s1 $dir
        $env:DOTNET_ROOT = $DOTNET_ROOT
        $p = Start-Process -FilePath $ins.File -ArgumentList $ins.Args -PassThru -WindowStyle Hidden
        $expected = (Tree $payload1).Count
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (-not $p.HasExited -and $sw.Elapsed.TotalSeconds -lt 120) {
            if ((@(Dir-Files $dir).Count) -ge ($expected * 0.4)) { break }
            Start-Sleep -Milliseconds 100
        }
        $killedAt = @(Dir-Files $dir).Count
        $wasRunning = -not $p.HasExited
        try { & taskkill.exe /F /T /PID $p.Id 2>&1 | Out-Null } catch { }
        Start-Sleep -Seconds 2
        $partial = @(Dir-Files $dir).Count
        $entryAfterKill = (@(Find-Entries).Count -gt 0)
        $again = Run-Timed $ins.File $ins.Args $StepTimeout
        $cmp = Compare-Tree $payload1 $dir
        $r.killed = [ordered]@{ killed_while_running = $wasRunning; files_when_killed = $partial; of = $expected; entry_after_kill = $entryAfterKill; rerun_exit = $again.ExitCode; rerun_seconds = $again.Seconds; rerun_identical = $cmp.Identical }
        Write-Host ("   killed with {0}/{1} files, entry {2}; second run exit {3}, identical {4}" -f $partial, $expected, $entryAfterKill, $again.ExitCode, $cmp.Identical)
        $un = & $def.Uninstall $s1 $dir; [void](Run-Timed $un.File $un.Args 120); [void](Wait-Removed $dir 60)
        Force-Clean $dir
    }

    # F. files the user added to the install folder, then removal
    Write-Host "== ${Tool}: user files and removal"
    $ins = & $def.Install $s1 $dir; [void](Run-Timed $ins.File $ins.Args $StepTimeout)
    Set-Content -Encoding ASCII (Join-Path $dir 'user-notes.txt') 'mine'
    New-Item -ItemType Directory -Force (Join-Path $dir 'user-data') | Out-Null
    Set-Content -Encoding ASCII (Join-Path $dir 'user-data\data.db') 'mine'
    $un = & $def.Uninstall $s1 $dir; [void](Run-Timed $un.File $un.Args 120); [void](Wait-Removed $dir 90)
    Start-Sleep -Seconds 2
    $kept = (Test-Path (Join-Path $dir 'user-notes.txt')) -and (Test-Path (Join-Path $dir 'user-data\data.db'))
    $r.user_files = [ordered]@{ kept_after_removal = $kept }
    Write-Host ("   user files kept: {0}" -f $kept)
    Force-Clean $dir

    $r.finished = (Get-Date).ToString('s')
    $r
}

foreach ($tool in $Tools) {
    $res = Test-Tool $tool
    $path = Join-Path $Results "$tool.json"
    $res | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $path
    Write-Host "wrote $path"
}
