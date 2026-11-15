# End-to-end test of a real setup file: silent install, an update that hits
# a locked file (and must leave the old version intact), a real update, a
# refused downgrade, a damaged setup file, and the silent uninstall.
#
#   cargo build -p bach-cli -p bach-setup -p hello
#   powershell -ExecutionPolicy Bypass -File scripts\e2e.ps1 [-Profile debug|release]
#
# It installs a throwaway app ("Bach E2E", its own id) into a temp folder
# and removes it again; nothing else on the computer is touched.

param([string]$Profile = 'debug')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$bach = Join-Path $root "target\$Profile\bach.exe"
$hello = Join-Path $root "target\$Profile\hello.exe"
foreach ($file in $bach, $hello) { if (-not (Test-Path $file)) { throw "Build first: $file is missing." } }

$work = Join-Path ([IO.Path]::GetTempPath()) ("bach-e2e-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$install = Join-Path $work 'Installed\BachE2E'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BachE2E'
$failures = 0

function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "  ok    $what" } else { Write-Host "  FAIL  $what" -ForegroundColor Red; $script:failures++ }
}

function Build([string]$version, [string[]]$extra) {
    $app = Join-Path $work "app-$version"
    New-Item -ItemType Directory -Force -Path $app | Out-Null
    Copy-Item $hello (Join-Path $app 'hello.exe')
    foreach ($name in $extra) { Set-Content -Path (Join-Path $app $name) -Value "$name $version" }
    @"
[app]
name = "Bach E2E"
id = "BachE2E"
version = "$version"
publisher = "Bach"
executable = "hello.exe"
[files]
source = "app-$version"
[installer]
output = "out/E2E-Setup-$version.exe"
desktop_shortcut = false
compression = "fast"
"@ | Set-Content -Encoding utf8 (Join-Path $work "bach-$version.toml")
    & $bach build (Join-Path $work "bach-$version.toml") | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bach build $version failed." }
    return Join-Path $work "out\E2E-Setup-$version.exe"
}

function Run([string]$exe, [string[]]$arguments) {
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    return $process.ExitCode
}

# Like Run, but waits only for the program itself, not for what it started
# (Start-Process -Wait also waits for every program the first one launched).
function Run-Alone([string]$exe, [string[]]$arguments) {
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $process.WaitForExit()
    return $process.ExitCode
}

function Installed-Version { (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).DisplayVersion }

if (Test-Path $key) { throw "BachE2E is already installed; remove it first." }
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    $v1 = Build '1.0.0' @('readme.txt', 'old.txt')
    $v2 = Build '2.0.0' @('readme.txt', 'new.txt')

    Write-Host 'Install into a folder that holds someone else''s files'
    New-Item -ItemType Directory -Force -Path $install | Out-Null
    Set-Content -Path (Join-Path $install 'readme.txt') -Value 'USER OWN SETTINGS'
    Set-Content -Path (Join-Path $install 'other-app.dll') -Value 'another app'
    Check ((Run $v1 @('--silent', '--dir', "`"$install`"")) -eq 1) 'refused: readme.txt is not from this app'
    Check ((Get-Content (Join-Path $install 'readme.txt')) -eq 'USER OWN SETTINGS') 'the foreign file is untouched'
    Check (-not (Installed-Version)) 'nothing was registered'
    Check ((Run $v1 @('--silent', '--overwrite', '--dir', "`"$install`"")) -eq 0) '--overwrite installs anyway'
    Set-Content -Path (Join-Path $install 'readme.txt') -Value 'edited after install'
    Check ((Run (Join-Path $install 'uninstall.exe') @('--silent')) -eq 0) 'uninstall exits 0'
    Check ((Get-Content (Join-Path $install 'readme.txt')) -eq 'edited after install') 'a file changed after install is kept'
    Check (Test-Path (Join-Path $install 'other-app.dll')) 'the other app''s file stays'
    Check (-not (Test-Path (Join-Path $install 'hello.exe'))) 'unchanged app files are removed'
    Remove-Item -Recurse -Force $install

    Write-Host 'Install 1.0.0'
    Check ((Run $v1 @('--silent', '--dir', "`"$install`"")) -eq 0) 'exits 0'
    Check ((Installed-Version) -eq '1.0.0') 'listed in Settings > Apps as 1.0.0'
    Check (Test-Path (Join-Path $install 'uninstall.exe')) 'uninstaller written'
    Check (Test-Path (Join-Path $install 'old.txt')) 'files installed'

    Write-Host 'Update while a file is locked'
    $lock = [IO.File]::Open((Join-Path $install 'readme.txt'), 'Open', 'Read', 'None')
    try { $code = Run $v2 @('--silent') } finally { $lock.Close() }
    Check ($code -eq 1) 'exits 1'
    Check ((Installed-Version) -eq '1.0.0') 'still 1.0.0'
    Check ((Get-Content (Join-Path $install 'readme.txt')) -eq 'readme.txt 1.0.0') 'old file intact'
    Check (-not (Test-Path (Join-Path $install 'new.txt'))) 'no new file left behind'
    Check (-not (Get-ChildItem $install -Recurse -Filter '*.bach-*')) 'no .bach-new or .bach-old left'

    Write-Host 'Update to 2.0.0'
    Check ((Run $v2 @('--silent')) -eq 0) 'exits 0'
    Check ((Installed-Version) -eq '2.0.0') 'listed as 2.0.0'
    Check (-not (Test-Path (Join-Path $install 'old.txt'))) 'file 2.0.0 dropped is gone'
    Check (Test-Path (Join-Path $install 'new.txt')) 'new file installed'

    Write-Host 'Downgrade'
    Check ((Run $v1 @('--silent')) -eq 1) 'refused without --allow-downgrade'
    Check ((Installed-Version) -eq '2.0.0') 'still 2.0.0'

    Write-Host 'Damaged setup file'
    $damaged = Join-Path $work 'out\Damaged.exe'
    Copy-Item $v1 $damaged
    $bytes = [IO.File]::ReadAllBytes($damaged)
    $end = $bytes.Length
    $padding = [BitConverter]::ToUInt32($bytes, $end - 28)
    $archive = [BitConverter]::ToUInt64($bytes, $end - 24)
    $logo = [BitConverter]::ToUInt64($bytes, $end - 16)
    $manifest = [BitConverter]::ToUInt64($bytes, $end - 8)
    $start = $end - 40 - $padding - $manifest - $logo - $archive
    $bytes[[int]($start + $archive / 2)] = $bytes[[int]($start + $archive / 2)] -bxor 0xFF
    [IO.File]::WriteAllBytes($damaged, $bytes)
    Check ((Run $damaged @('--silent', '--allow-downgrade')) -eq 1) 'refused'
    Check ((Installed-Version) -eq '2.0.0') 'install untouched'

    Write-Host 'Uninstall'
    Set-Content -Path (Join-Path $install 'mine.txt') -Value 'a file the user made'
    Check ((Run (Join-Path $install 'uninstall.exe') @('--silent')) -eq 0) 'exits 0'
    Check (-not (Test-Path $key)) 'gone from Settings > Apps'
    Check (Test-Path (Join-Path $install 'mine.txt')) "the user's own file stays"
    Check (-not (Test-Path (Join-Path $install 'hello.exe'))) 'app files gone'
} finally {
    if (Test-Path $key) {
        $uninstaller = Join-Path $install 'uninstall.exe'
        if (Test-Path $uninstaller) { Run $uninstaller @('--silent') | Out-Null }
    }
    Start-Sleep -Seconds 3
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# Updates: the staged update that waits for the app to close, the running
# uninstaller that replaces itself, an update cut short, and the task that
# keeps an app current. Offline: the feed is never asked (its address does
# not exist), only what the installed uninstaller does with a download that
# is already waiting.
# ---------------------------------------------------------------------------
$id = 'BachE2EUpdates'
$updatesKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$id"
$updatesInstall = Join-Path $work 'Installed\BachE2EUpdates'
$staging = Join-Path $env:LOCALAPPDATA "Bach\updates\$id"

function BuildU([string]$version, [string]$appId = $id, [string]$toml = '') {
    $app = Join-Path $work "app-$appId-$version"
    New-Item -ItemType Directory -Force -Path $app | Out-Null
    Copy-Item $hello (Join-Path $app 'hello.exe')
    Set-Content -Path (Join-Path $app 'readme.txt') -Value "readme.txt $version"
    @"
[app]
name = "Bach E2E Updates"
id = "$appId"
version = "$version"
publisher = "Bach"
executable = "hello.exe"
[files]
source = "app-$appId-$version"
[installer]
output = "out/$appId-Setup-$version.exe"
desktop_shortcut = false
compression = "fast"
[updates]
feed = "https://example.invalid/bach-e2e/latest.json"
$toml
"@ | Set-Content -Encoding utf8 (Join-Path $work "bach-$appId-$version.toml")
    & $bach build (Join-Path $work "bach-$appId-$version.toml") | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bach build $appId $version failed." }
    return Join-Path $work "out\$appId-Setup-$version.exe"
}

# Puts a setup file where a background download would have left it.
function Stage([string]$setup, [string]$version, [string]$hash = '') {
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    $name = "Setup-$version.exe"
    Copy-Item $setup (Join-Path $staging $name) -Force
    if (-not $hash) { $hash = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLower() }
    $marker = @{ version = $version; sha256 = $hash; file = $name; notes = 'E2E' } | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $staging 'staged.json'), $marker)
}

function Task-Exists { cmd /c "schtasks /Query /TN ""Bach\$id"" >nul 2>&1"; return ($LASTEXITCODE -eq 0) }
function Updates-Version { (Get-ItemProperty -Path $updatesKey -ErrorAction SilentlyContinue).DisplayVersion }

if (Test-Path $updatesKey) { throw "$id is already installed; remove it first." }
try {
    $u1 = BuildU '1.0.0'
    $u2 = BuildU '2.0.0'
    $u3 = BuildU '3.0.0'
    $uninstaller = Join-Path $updatesInstall 'uninstall.exe'

    Write-Host 'Install 1.0.0 with automatic updates'
    Check ((Run $u1 @('--silent', '--dir', "`"$updatesInstall`"")) -eq 0) 'exits 0'
    Check (Task-Exists) 'the update task exists'

    Write-Host 'An update waiting while the app is open'
    Stage $u2 '2.0.0'
    $lock = [IO.File]::Open((Join-Path $updatesInstall 'hello.exe'), 'Open', 'Read', 'None')
    try { $code = Run $uninstaller @('--apply-update') } finally { $lock.Close() }
    Check ($code -eq 1) 'exits 1'
    Check ((Updates-Version) -eq '1.0.0') 'still 1.0.0'
    Check (Test-Path (Join-Path $staging 'staged.json')) 'the download stays for the next try'

    Write-Host 'The app closed: the waiting update installs'
    Check ((Run $uninstaller @('--apply-update')) -eq 0) 'exits 0'
    Check ((Updates-Version) -eq '2.0.0') 'now 2.0.0'
    Check (-not (Test-Path $staging)) 'the finished download is deleted'
    Check ((Get-Content (Join-Path $updatesInstall 'readme.txt')) -eq 'readme.txt 2.0.0') 'files are the new ones'
    Check (-not (Test-Path "$uninstaller.bach-new")) 'no half-written uninstaller'
    Start-Sleep -Seconds 3
    Check (-not (Test-Path "$uninstaller.bach-old")) 'the old uninstaller is cleaned up after it exited'
    Check ([bool]((& $bach inspect $uninstaller) -match 'Uninstaller for Bach E2E Updates 2\.0\.0')) 'the uninstaller is the new one'

    Write-Host 'It waits for the app to end (--wait-for), then starts it again (--launch)'
    Stage $u3 '3.0.0'
    $sleeper = Start-Process -FilePath "$env:SystemRoot\System32\ping.exe" -ArgumentList '-n', '5', '127.0.0.1' -PassThru -WindowStyle Hidden
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $code = Run-Alone $uninstaller @('--apply-update', '--wait-for', $sleeper.Id, '--launch')
    $watch.Stop()
    Check ($code -eq 0) 'exits 0'
    Check ($watch.Elapsed.TotalSeconds -ge 2.5) ("waited for the other program ($([int]$watch.Elapsed.TotalSeconds) s)")
    Check ($watch.Elapsed.TotalSeconds -lt 30) 'and then installed quickly'
    Check ((Updates-Version) -eq '3.0.0') 'now 3.0.0'
    # Windows may take a moment to start a program it has just scanned.
    $running = $null
    for ($try = 0; $try -lt 40 -and -not $running; $try++) {
        Start-Sleep -Milliseconds 500
        $running = Get-Process -Name hello -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$updatesInstall*" }
    }
    Check ([bool]$running) 'the app was started again'
    $running | Stop-Process -Force -ErrorAction SilentlyContinue

    Write-Host 'Downloads that must never run'
    $damaged = Join-Path $work 'out\DamagedU.exe'
    $other = BuildU '9.0.0' 'BachE2EOther'
    Stage $u3 '4.0.0' ('0' * 64)
    Check ((Run $uninstaller @('--apply-update')) -eq 1) 'a file that is not what the feed said: refused'
    Check (-not (Test-Path $staging)) 'and thrown away'
    Stage $other '9.0.0'
    Check ((Run $uninstaller @('--apply-update')) -eq 1) 'another app''s setup file: refused'
    Check ((Updates-Version) -eq '3.0.0') 'the install is untouched'
    Stage $u1 '1.0.0'
    Check ((Run $uninstaller @('--apply-update')) -eq 0) 'an older version waiting is just dropped'
    Check (-not (Test-Path $staging)) 'and cleaned up'

    Write-Host 'An update cut short is undone'
    Copy-Item $u1 $damaged
    $bytes = [IO.File]::ReadAllBytes($damaged)
    $end = $bytes.Length
    $padding = [BitConverter]::ToUInt32($bytes, $end - 28)
    $archive = [BitConverter]::ToUInt64($bytes, $end - 24)
    $logo = [BitConverter]::ToUInt64($bytes, $end - 16)
    $manifest = [BitConverter]::ToUInt64($bytes, $end - 8)
    $start = $end - 40 - $padding - $manifest - $logo - $archive
    $bytes[[int]($start + $archive / 2)] = $bytes[[int]($start + $archive / 2)] -bxor 0xFF
    [IO.File]::WriteAllBytes($damaged, $bytes)
    $readme = Join-Path $updatesInstall 'readme.txt'
    Move-Item $readme "$readme.bach-old"
    Set-Content "$readme.bach-new" 'half written'
    [IO.File]::WriteAllText((Join-Path $updatesInstall '.bach-update'), "R$readme`n")
    Check ((Run $damaged @('--silent', '--allow-downgrade')) -eq 1) 'the next setup run (a damaged one) still fails'
    Check ((Get-Content $readme) -eq 'readme.txt 3.0.0') 'but the file the cut-short update moved is back'
    Check (-not (Get-ChildItem $updatesInstall -Force -Recurse | Where-Object { $_.Name -like '*.bach-*' })) 'no .bach-* files and no journal left'

    Write-Host 'Unticking automatic updates takes the task away'
    $u4 = BuildU '4.0.0'
    Check ((Run $u4 @('--silent', '--without', 'auto-update')) -eq 0) 'exits 0'
    Check (-not (Task-Exists)) 'the update task is gone'
    Check ((Run $u4 @('--silent', '--with', 'auto-update', '--allow-downgrade')) -eq 0) 'ticked again'
    Check (Task-Exists) 'the update task is back'

    Write-Host 'Uninstall removes the task and any waiting download'
    Stage $u3 '5.0.0'
    Check ((Run $uninstaller @('--silent')) -eq 0) 'exits 0'
    Check (-not (Test-Path $updatesKey)) 'gone from Settings > Apps'
    Check (-not (Task-Exists)) 'the update task is gone'
    Check (-not (Test-Path $staging)) 'the waiting download is gone'
} finally {
    cmd /c "schtasks /Delete /TN ""Bach\$id"" /F >nul 2>&1"
    Get-Process -Name hello -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$updatesInstall*" } | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Test-Path $updatesKey) {
        $uninstaller = Join-Path $updatesInstall 'uninstall.exe'
        if (Test-Path $uninstaller) { Run $uninstaller @('--silent') | Out-Null }
    }
    Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# A protected app: encrypted files, an access key that is checked before
# anything is touched, kept sealed for updates and forgotten on removal, and
# a program the app asks setup to run after install and before removal.
# ---------------------------------------------------------------------------
Write-Host 'A protected app with run steps'
$pid2 = 'BachE2EProtected'
$protKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$pid2"
$protInstall = Join-Path $work 'Installed\BachE2EProtected'
$access = 'HKCU:\Software\Bach\Access'
$secret = 'E2E-private-words-ZX91-launch'
if (Test-Path $protKey) { throw "$pid2 is already installed; remove it first." }
$protDir = Join-Path $work 'prot'
New-Item -ItemType Directory -Force -Path (Join-Path $protDir 'app') | Out-Null
Copy-Item $hello (Join-Path $protDir 'app\hello.exe')
Copy-Item "$env:SystemRoot\System32\cmd.exe" (Join-Path $protDir 'app\tool.exe')
Set-Content -Path (Join-Path $protDir 'app\notes.txt') -Value $secret
@"
[app]
name = "Bach E2E Protected"
id = "$pid2"
version = "1.0.0"
publisher = "Bach"
executable = "hello.exe"
[files]
source = "app"
[installer]
compression = "fast"
desktop_shortcut = false
[access]
[updates]
feed = "https://example.invalid/bach-e2e/latest.json"
auto = false
[[run]]
name = "Install marker"
file = "tool.exe"
arguments = "/c echo installed> marker-install.txt"
[[run]]
name = "Removal marker"
file = "tool.exe"
arguments = '/c echo removing> "%TEMP%\bach-e2e-removal.txt"'
when = "uninstall"
"@ | Set-Content -Encoding utf8 (Join-Path $protDir 'bach.toml')
$goodKey = (& $bach key).Trim()
$badKey = (& $bach key).Trim()
Set-Content -NoNewline -Path (Join-Path $protDir 'good.txt') -Value $goodKey
Set-Content -NoNewline -Path (Join-Path $protDir 'bad.txt') -Value $badKey
try {
    $env:BACH_ACCESS_KEY = $goodKey
    & $bach build (Join-Path $protDir 'bach.toml') | Out-Null
    Remove-Item Env:\BACH_ACCESS_KEY
    Check ($LASTEXITCODE -eq 0) 'builds with the key from the environment'
    $setup = Join-Path $protDir 'dist\Bach E2E Protected-Setup-1.0.0.exe'
    $raw = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($setup))
    Check (-not $raw.Contains($secret)) 'the packaged file is not readable in the setup file'
    Check (-not $raw.Contains($goodKey)) 'the key is not in the setup file'

    Write-Host '  keys'
    Check ((Run $setup @('--silent', '--dir', "`"$protInstall`"")) -eq 2) 'no key: a command-line mistake (2)'
    Check ((Run $setup @('--silent', '--dir', "`"$protInstall`"", '--key-file', (Join-Path $protDir 'bad.txt'))) -eq 1) 'a wrong key: fails (1)'
    Check (-not (Test-Path $protInstall)) 'and touched nothing'
    Check (-not (Test-Path $protKey)) 'and registered nothing'

    Write-Host '  install'
    Check ((Run $setup @('--silent', '--dir', "`"$protInstall`"", '--key-file', (Join-Path $protDir 'good.txt'))) -eq 0) 'the right key installs'
    Check ((Get-Content (Join-Path $protInstall 'notes.txt')) -eq $secret) 'the file is decrypted intact'
    Check (Test-Path (Join-Path $protInstall 'marker-install.txt')) 'the run step ran, in the install folder'
    Check ($null -ne (Get-ItemProperty $access -Name $pid2 -ErrorAction SilentlyContinue).$pid2) 'the key is kept for updates'
    $kept = [Text.Encoding]::GetEncoding(28591).GetString((Get-ItemProperty $access).$pid2)
    Check (-not $kept.Contains($goodKey)) 'and what is kept is sealed, not the key'

    Write-Host '  a tampered setup file'
    $tampered = Join-Path $protDir 'tampered.exe'
    Copy-Item $setup $tampered
    $bytes = [IO.File]::ReadAllBytes($tampered)
    $end = $bytes.Length
    $padding = [BitConverter]::ToUInt32($bytes, $end - 28)
    $archive = [BitConverter]::ToUInt64($bytes, $end - 24)
    $logo = [BitConverter]::ToUInt64($bytes, $end - 16)
    $manifestLen = [BitConverter]::ToUInt64($bytes, $end - 8)
    $start = $end - 40 - $padding - $manifestLen - $logo - $archive
    $bytes[[int]($start + $archive / 2)] = $bytes[[int]($start + $archive / 2)] -bxor 0xFF
    [IO.File]::WriteAllBytes($tampered, $bytes)
    Check ((Run $tampered @('--silent', '--key-file', (Join-Path $protDir 'good.txt'))) -eq 1) 'refused, even with the right key'
    Check ((Get-Content (Join-Path $protInstall 'notes.txt')) -eq $secret) 'the install is untouched'

    Write-Host '  uninstall'
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $env:TEMP 'bach-e2e-removal.txt')
    Check ((Run (Join-Path $protInstall 'uninstall.exe') @('--silent')) -eq 0) 'exits 0'
    Check (Test-Path (Join-Path $env:TEMP 'bach-e2e-removal.txt')) 'the before-removal step ran'
    Check (-not (Test-Path $protKey)) 'gone from Settings > Apps'
    Check ($null -eq (Get-ItemProperty $access -Name $pid2 -ErrorAction SilentlyContinue).$pid2) 'the kept key is forgotten'
} finally {
    Remove-Item Env:\BACH_ACCESS_KEY -ErrorAction SilentlyContinue
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $env:TEMP 'bach-e2e-removal.txt')
    if (Test-Path $protKey) {
        $uninstaller = Join-Path $protInstall 'uninstall.exe'
        if (Test-Path $uninstaller) { Run $uninstaller @('--silent') | Out-Null }
    }
    Remove-ItemProperty -Path $access -Name $pid2 -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# Building for a release: the tag is the version, GitHub names the file.
# ---------------------------------------------------------------------------
Write-Host 'A release build: version from the tag, feed from the config'
$github = Join-Path $work 'github'
New-Item -ItemType Directory -Force -Path (Join-Path $github 'app') | Out-Null
Copy-Item $hello (Join-Path $github 'app\hello.exe')
@"
[app]
name = "My Great App"
version = "0.0.1"
publisher = "Bach"
executable = "hello.exe"
[files]
source = "app"
[installer]
compression = "fast"
[updates]
github = "usebach/e2e"
"@ | Set-Content -Encoding utf8 (Join-Path $github 'bach.toml')
Push-Location $github
try {
    & $bach build bach.toml --app-version v4.2.0 | Out-Null
    Check ($LASTEXITCODE -eq 0) 'builds with --app-version v4.2.0'
    $file = Join-Path $github 'dist\My-Great-App-Setup-4.2.0.exe'
    Check (Test-Path $file) 'the setup file is named the way GitHub will serve it'
    $feed = Get-Content (Join-Path $github 'dist\latest.json') -Raw | ConvertFrom-Json
    Check ($feed.version -eq '4.2.0') 'the feed says 4.2.0'
    Check ($feed.url -eq 'https://github.com/usebach/e2e/releases/download/v4.2.0/My-Great-App-Setup-4.2.0.exe') 'and points at the tag''s own download'
    Check ($feed.sha256 -eq (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower()) 'with the file''s SHA-256'
    Check ($feed.size -eq (Get-Item $file).Length) 'and its size'
    Set-Content -Path (Join-Path $github 'notes.md') -Value 'What is new in 4.2.0'
    & $bach feed $file --notes-file (Join-Path $github 'notes.md') | Out-Null
    $feed = Get-Content (Join-Path $github 'dist\latest.json') -Raw | ConvertFrom-Json
    Check ($feed.notes -eq 'What is new in 4.2.0') 'bach feed writes it again with the notes, link from bach.toml'
    Check ($feed.url -like '*/v4.2.0/*') 'and the same link'
} finally { Pop-Location }

# ---------------------------------------------------------------------------
# Update patches: bach build --patch-from writes a setup file that carries
# only the files that changed. It updates the version it was made for, leaves
# the other files alone, and refuses to run on anything else.
# ---------------------------------------------------------------------------
$patchId = 'BachE2EPatch'
$patchKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$patchId"
$patchWork = Join-Path ([IO.Path]::GetTempPath()) ("bach-e2e-patch-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$patchInstall = Join-Path $patchWork 'Installed\BachE2EPatch'
function Patch-Version { (Get-ItemProperty -Path $patchKey -ErrorAction SilentlyContinue).DisplayVersion }

function BuildP([string]$version, [string]$changing, [string[]]$extra = @()) {
    $app = Join-Path $patchWork "app-$version"
    New-Item -ItemType Directory -Force -Path $app | Out-Null
    Copy-Item $hello (Join-Path $app 'hello.exe')
    Set-Content -Path (Join-Path $app 'stable.txt') -Value 'never changes'
    # A big file that stays the same: what a patch is for. (A patch carries the
    # setup runtime too, so it only pays off for apps with something big.)
    $big = Join-Path $patchWork 'big.bin'
    if (-not (Test-Path $big)) {
        $bytes = New-Object byte[] (30MB)
        (New-Object Random 42).NextBytes($bytes)
        [IO.File]::WriteAllBytes($big, $bytes)
    }
    Copy-Item $big (Join-Path $app 'big.bin')
    Set-Content -Path (Join-Path $app 'changing.txt') -Value $changing
    @"
[app]
name = "Bach E2E Patch"
id = "$patchId"
version = "$version"
publisher = "Bach"
executable = "hello.exe"
[files]
source = "app-$version"
[installer]
output = "out/P-Setup-$version.exe"
desktop_shortcut = false
compression = "fast"
[updates]
feed = "https://example.invalid/bach-e2e/latest.json"
download_url = "https://example.invalid/bach-e2e/{file}"
"@ | Set-Content -Encoding utf8 (Join-Path $patchWork "bach-$version.toml")
    & $bach build (Join-Path $patchWork "bach-$version.toml") @extra | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "bach build patch $version failed." }
    return Join-Path $patchWork "out\P-Setup-$version.exe"
}

if (Test-Path $patchKey) { throw "$patchId is already installed; remove it first." }
New-Item -ItemType Directory -Force -Path $patchWork | Out-Null
try {
    Write-Host 'Build a patch'
    $p1 = BuildP '1.0.0' 'first'
    $p2 = BuildP '2.0.0' 'second' @('--patch-from', $p1)
    $patch = Join-Path $patchWork 'out\P-Setup-2.0.0-patch-from-1.0.0.exe'
    Check (Test-Path $patch) 'the patch is written next to the setup file'
    Check ((Get-Item $patch).Length -lt ((Get-Item $p2).Length / 2)) 'and is much smaller: the unchanged big file isn''t in it'
    $feed = Get-Content (Join-Path $patchWork 'out\latest.json') -Raw | ConvertFrom-Json
    Check ($feed.delta.from -eq '1.0.0') 'the feed offers it to computers on 1.0.0'
    Check ($feed.delta.sha256 -eq (Get-FileHash -Algorithm SHA256 $patch).Hash.ToLower()) 'with the patch''s own SHA-256'
    Check ($feed.sha256 -eq (Get-FileHash -Algorithm SHA256 $p2).Hash.ToLower()) 'and still the full file''s'

    Write-Host 'The patch updates the version it was made for'
    Check ((Run $p1 @('--silent', '--dir', "`"$patchInstall`"")) -eq 0) 'installs 1.0.0'
    Check ((Run $patch @('--silent')) -eq 0) 'the patch exits 0'
    Check ((Patch-Version) -eq '2.0.0') 'listed as 2.0.0'
    Check ((Get-Content (Join-Path $patchInstall 'changing.txt')) -eq 'second') 'the changed file is new'
    Check ((Get-Content (Join-Path $patchInstall 'stable.txt')) -eq 'never changes') 'the unchanged file is as it was'
    Check ((Get-FileHash -Algorithm SHA256 (Join-Path $patchInstall 'big.bin')).Hash -eq (Get-FileHash -Algorithm SHA256 (Join-Path $patchWork 'big.bin')).Hash) 'and so is the big file'
    Check (Test-Path (Join-Path $patchInstall 'hello.exe')) 'and the program'
    Check (-not (Get-ChildItem $patchInstall -Recurse -Filter '*.bach-*')) 'nothing left behind'

    Write-Host 'The patch refuses an install it wasn''t made for'
    Check ((Run (Join-Path $patchInstall 'uninstall.exe') @('--silent')) -eq 0) 'removes 2.0.0'
    Check ((Run $p1 @('--silent', '--dir', "`"$patchInstall`"")) -eq 0) 'installs 1.0.0 again'
    [IO.File]::AppendAllText((Join-Path $patchInstall 'big.bin'), 'changed by someone')
    Check ((Run $patch @('--silent')) -eq 1) 'exits 1 when a file it relies on has changed'
    Check ((Patch-Version) -eq '1.0.0') 'still 1.0.0'
    Check ((Get-Content (Join-Path $patchInstall 'changing.txt')) -eq 'first') 'nothing was half updated'
    Check (-not (Get-ChildItem $patchInstall -Recurse -Filter '*.bach-*')) 'and nothing is left behind'

    Write-Host 'The patch is no installer'
    Check ((Run (Join-Path $patchInstall 'uninstall.exe') @('--silent')) -eq 0) 'removes it'
    Check ((Run $patch @('--silent', '--dir', "`"$patchInstall`"")) -eq 1) 'exits 1 on a computer that has nothing installed'
    Check (-not (Test-Path $patchKey)) 'and installs nothing'
} finally {
    if (Test-Path $patchKey) {
        $uninstaller = Join-Path $patchInstall 'uninstall.exe'
        if (Test-Path $uninstaller) { Run $uninstaller @('--silent') | Out-Null }
    }
    Start-Sleep -Seconds 3
    Remove-Item -Recurse -Force $patchWork -ErrorAction SilentlyContinue
}

if ($failures -gt 0) { throw "$failures end-to-end check(s) failed." }
Write-Host 'All end-to-end checks passed.' -ForegroundColor Green
