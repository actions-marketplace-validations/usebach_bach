# Written by `bach test`. It runs inside Windows Sandbox (or, with -Local, on this
# computer): it installs the setup file silently, checks what the install did,
# starts the app, removes it again and reports what was left behind.
#
# Everything it needs comes from params.json next to it, never from the text of
# this script. The report is out\report.json.
#
# Keep this file ASCII only: Windows PowerShell 5.1 reads a script without a byte
# order mark in the computer's ANSI code page.

param([string]$Work = 'C:\bach-test', [switch]$Local)
$ErrorActionPreference = 'Continue'
$in = Join-Path $Work 'in'
$out = Join-Path $Work 'out'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$params = [IO.File]::ReadAllText((Join-Path $in 'params.json')) | ConvertFrom-Json
$steps = New-Object System.Collections.ArrayList
$left = [ordered]@{ files = @(); registry = @(); other = @() }
$facts = [ordered]@{ install_dir = ''; version = ''; files_expected = 0; files_found = 0 }
$started = Get-Date

function Step([string]$name, [bool]$ok, [string]$detail) {
    [void]$steps.Add([ordered]@{ name = $name; ok = $ok; detail = $detail })
}

function Seconds($since) { [math]::Round(((Get-Date) - $since).TotalSeconds, 1) }

function New-Set { New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase) }

# The folders and keys an install may leave things in, two levels deep.
function Snapshot-Files {
    $set = New-Set
    $roots = @(
        $env:LOCALAPPDATA, $env:APPDATA, $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramData,
        [Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory'),
        [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('CommonPrograms'),
        [Environment]::GetFolderPath('Startup'), [Environment]::GetFolderPath('MyDocuments')
    )
    # Windows and the programs that come with it write here all the time; an
    # install of an app doesn't, so it would only be noise in the report.
    $noise = @(
        $env:TEMP, (Join-Path $env:LOCALAPPDATA 'Temp'), (Join-Path $env:LOCALAPPDATA 'Microsoft'), (Join-Path $env:LOCALAPPDATA 'Packages'),
        (Join-Path $env:LOCALAPPDATA 'CrashDumps'), (Join-Path $env:LOCALAPPDATA 'D3DSCache'), (Join-Path $env:LOCALAPPDATA 'PeerDistRepub'),
        (Join-Path $env:APPDATA 'Microsoft'), (Join-Path $env:ProgramData 'Microsoft'), (Join-Path $env:ProgramData 'Package Cache'),
        (Join-Path $env:ProgramFiles 'WindowsApps'), (Join-Path $env:ProgramFiles 'Microsoft')
    ) | Where-Object { $_ }
    foreach ($root in $roots) {
        if (-not $root -or -not (Test-Path -LiteralPath $root)) { continue }
        Get-ChildItem -LiteralPath $root -Force -Depth 1 -ErrorAction SilentlyContinue | ForEach-Object {
            $path = $_.FullName
            $skip = $false
            foreach ($prefix in $noise) {
                if ($path.Equals($prefix, [StringComparison]::OrdinalIgnoreCase) -or $path.StartsWith($prefix + '\', [StringComparison]::OrdinalIgnoreCase)) { $skip = $true; break }
            }
            if (-not $skip) { [void]$set.Add($path) }
        }
    }
    return , $set
}

function Snapshot-Registry {
    $set = New-Set
    foreach ($root in 'HKCU:\Software', 'HKLM:\SOFTWARE') {
        Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue | ForEach-Object {
            [void]$set.Add($_.Name)
            Get-ChildItem -LiteralPath $_.PSPath -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add($_.Name) }
        }
    }
    foreach ($root in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
        Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add($_.Name) }
    }
    return , $set
}

function Snapshot-Other {
    $set = New-Set
    $environment = Get-ItemProperty -LiteralPath 'HKCU:\Environment' -ErrorAction SilentlyContinue
    if ($environment) {
        foreach ($property in $environment.PSObject.Properties) {
            if ($property.Name -notlike 'PS*') { [void]$set.Add("environment variable $($property.Name) = $($property.Value)") }
        }
    }
    $run = Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
    if ($run) {
        foreach ($property in $run.PSObject.Properties) {
            if ($property.Name -notlike 'PS*') { [void]$set.Add("starts with Windows: $($property.Name)") }
        }
    }
    Get-ScheduledTask -ErrorAction SilentlyContinue | ForEach-Object { [void]$set.Add("scheduled task $($_.TaskPath)$($_.TaskName)") }
    return , $set
}

# What is in `after` and not in `before`, and for a new folder the files in it.
function New-Entries($before, $after) {
    $found = New-Object System.Collections.ArrayList
    foreach ($path in $after) {
        if ($before.Contains($path)) { continue }
        [void]$found.Add($path)
        if (Test-Path -LiteralPath $path -PathType Container) {
            Get-ChildItem -LiteralPath $path -Force -Recurse -File -ErrorAction SilentlyContinue | Select-Object -First 50 | ForEach-Object { [void]$found.Add($_.FullName) }
        }
    }
    return @($found | Select-Object -Unique)
}

function Find-Install {
    foreach ($root in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
        $key = Join-Path $root $params.id
        if (Test-Path -LiteralPath $key) { return Get-ItemProperty -LiteralPath $key }
    }
    return $null
}

function Run-Program([string]$file, [string[]]$arguments) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $process.WaitForExit()
    return $process.ExitCode
}

try {
    $beforeFiles = Snapshot-Files
    $beforeRegistry = Snapshot-Registry
    $beforeOther = Snapshot-Other

    # 1. Install.
    $setup = Join-Path $in $params.setup
    $arguments = @('--silent', '--log', (Join-Path $out 'install.log'))
    if ($params.with.Count -gt 0) { $arguments += @('--with', ($params.with -join ',')) }
    if ($params.without.Count -gt 0) { $arguments += @('--without', ($params.without -join ',')) }
    $since = Get-Date
    $code = Run-Program $setup $arguments
    Step 'Installed' ($code -eq 0) ("exit code $code in $(Seconds $since) s")
    $install = Find-Install
    $dir = ''
    if ($code -eq 0) {
        # 2. Registered under Settings > Apps, and the files are there.
        if ($install) {
            $dir = [string]$install.InstallLocation
            $facts.install_dir = $dir
            $facts.version = [string]$install.DisplayVersion
            $versionOk = ($facts.version -eq $params.version) -or $params.online
            Step 'Registered' $versionOk ("Settings > Apps lists $($install.DisplayName) $($facts.version)" + $(if ($versionOk) { '' } else { ", but the setup file is version $($params.version)" }))
        } else {
            Step 'Registered' $false "no Settings > Apps entry named $($params.id) was written"
        }
        if ($dir -and (Test-Path -LiteralPath $dir)) {
            $missing = New-Object System.Collections.ArrayList
            $facts.files_expected = @($params.files).Count
            foreach ($file in @($params.files)) {
                $path = Join-Path $dir ($file.path -replace '/', '\')
                $item = Get-Item -LiteralPath $path -ErrorAction SilentlyContinue
                if ($item -and $item.Length -eq [int64]$file.size) { $facts.files_found++ } else { [void]$missing.Add($file.path) }
            }
            $detail = "$($facts.files_found) of $($facts.files_expected) files are in $dir"
            if ($missing.Count -gt 0) { $detail += '; missing or the wrong size: ' + (($missing | Select-Object -First 5) -join ', ') }
            Step 'Files' ($missing.Count -eq 0 -or $params.online) $detail
            Step 'Uninstaller' (Test-Path -LiteralPath (Join-Path $dir 'uninstall.exe')) 'uninstall.exe is in the install folder'
        }

        # 3. Start the app.
        $program = if ($dir) { Join-Path $dir ($params.executable -replace '/', '\') } else { '' }
        if ($params.launch_seconds -gt 0 -and $program -and (Test-Path -LiteralPath $program)) {
            $since = Get-Date
            $process = Start-Process -FilePath $program -WorkingDirectory $dir -PassThru
            $deadline = $since.AddSeconds([double]$params.launch_seconds)
            while ((Get-Date) -lt $deadline -and -not $process.HasExited) { Start-Sleep -Milliseconds 250 }
            if ($process.HasExited) {
                $ended = $process.ExitCode
                Step 'Started' ($ended -eq 0) ("ended after $(Seconds $since) s with exit code $ended")
            } else {
                Step 'Started' $true "still running after $($params.launch_seconds) s"
                [void]$process.CloseMainWindow()
                Start-Sleep -Seconds 2
            }
            # Setup won't remove a program that is open.
            Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 500
        }

        # 4. Remove it, and wait for the copy of the uninstaller that does the work.
        $uninstaller = if ($dir) { Join-Path $dir 'uninstall.exe' } else { '' }
        if ($uninstaller -and (Test-Path -LiteralPath $uninstaller)) {
            $since = Get-Date
            $removed = Run-Program $uninstaller @('--silent', '--delete-data', '--log', (Join-Path $out 'uninstall.log'))
            $limit = (Get-Date).AddSeconds(120)
            while ((Get-Date) -lt $limit -and (Find-Install)) { Start-Sleep -Milliseconds 500 }
            $gone = -not (Find-Install)
            Step 'Removed' ($removed -eq 0 -and $gone) ("exit code $removed in $(Seconds $since) s" + $(if ($gone) { '' } else { ', but the Settings > Apps entry is still there' }))
        }
    }

    # 5. What is left.
    Start-Sleep -Seconds 1
    $left.files = @(New-Entries $beforeFiles (Snapshot-Files))
    $left.registry = @(New-Entries $beforeRegistry (Snapshot-Registry))
    $left.other = @(New-Entries $beforeOther (Snapshot-Other))
    $total = $left.files.Count + $left.registry.Count + $left.other.Count
    if ($code -eq 0) {
        Step 'Left behind' ($total -eq 0) $(if ($total -eq 0) { 'nothing' } else { "$($left.files.Count) files and folders, $($left.registry.Count) registry keys, $($left.other.Count) other changes" })
    }
} catch {
    Step 'Test script' $false $_.Exception.Message
}

$report = [ordered]@{
    version = 1
    seconds = (Seconds $started)
    steps = @($steps)
    left_behind = $left
    facts = $facts
}
$temporary = Join-Path $out 'report.partial'
($report | ConvertTo-Json -Depth 6) | Set-Content -Path $temporary -Encoding UTF8
Move-Item -Force -LiteralPath $temporary -Destination (Join-Path $out 'report.json')

# The sandbox is thrown away when it ends; ending it is what "done" means,
# unless the test was asked to keep it open.
if (-not $Local -and -not $params.keep -and $env:USERNAME -eq 'WDAGUtilityAccount') {
    Start-Sleep -Seconds 2
    shutdown.exe /s /t 3
}
