# Builds the app the installers are compared on: a real Electron app (the
# Electron 33 runtime plus about 10,000 files of node_modules, 355 MB), in
# three payloads:
#
#   payload-1.0.0   the app
#   payload-1.1.0   the next release: a few hundred changed files, some new,
#                   some removed (what an update has to cope with)
#   payload-tiny    one small program, to measure what an installer adds
#                   to an app by itself
#
#   powershell -ExecutionPolicy Bypass -File bench\make-payload.ps1 -Work D:\bench-work
#
# Needs bun and 7-Zip (7z.exe on the PATH or via scoop). The Electron
# runtime is downloaded once (about 110 MB). Nothing here touches the
# repository: everything goes under -Work.

param([Parameter(Mandatory)][string]$Work, [string]$TinyProgram = '')
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Work | Out-Null
$Work = (Resolve-Path $Work).Path

function Find-7z {
    $found = Get-Command 7z -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    $scoop = Join-Path $env:USERPROFILE 'scoop\apps\7zip\current\7z.exe'
    if (Test-Path $scoop) { return $scoop }
    throw '7-Zip is needed to unpack the Electron runtime (scoop install 7zip).'
}

# 1. The app's own files, from the registry.
$app = Join-Path $Work 'appsrc'
New-Item -ItemType Directory -Force $app | Out-Null
Set-Content -Encoding ASCII (Join-Path $app 'main.js') @'
const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => {
  const win = new BrowserWindow({ width: 400, height: 300 });
  win.loadURL('data:text/html,<h1>Bench app</h1>');
  setTimeout(() => app.quit(), 3000);
});
'@
Set-Content -Encoding ASCII (Join-Path $app 'package.json') @'
{
  "name": "benchapp",
  "version": "1.0.0",
  "main": "main.js",
  "private": true,
  "dependencies": {
    "lodash": "4.17.21",
    "moment": "2.30.1",
    "rxjs": "7.8.1",
    "three": "0.170.0",
    "typescript": "5.6.3",
    "webpack": "5.96.1",
    "@babel/core": "7.26.0"
  }
}
'@
Push-Location $app
bun install --production | Out-Null
Pop-Location

# 2. The Electron runtime. Its own installer script is skipped (it stops after
# the locales folder on some Node versions), so the zip is fetched with it
# and unpacked with 7-Zip.
$electron = Join-Path $Work 'electron'
$zipName = 'electron-v33.2.0-win32-x64.zip'
$zip = Join-Path $Work $zipName
if (-not (Test-Path $zip)) {
    Invoke-WebRequest "https://github.com/electron/electron/releases/download/v33.2.0/$zipName" -OutFile $zip
}
if (Test-Path $electron) { Remove-Item $electron -Recurse -Force }
& (Find-7z) x -y "-o$electron" $zip | Out-Null

# 3. payload-1.0.0
$v1 = Join-Path $Work 'payload-1.0.0'
if (Test-Path $v1) { Remove-Item $v1 -Recurse -Force }
Copy-Item $electron $v1 -Recurse
Rename-Item (Join-Path $v1 'electron.exe') 'BenchApp.exe'
$res = Join-Path $v1 'resources\app'
New-Item -ItemType Directory -Force $res | Out-Null
Copy-Item (Join-Path $app 'main.js'), (Join-Path $app 'package.json') $res
Copy-Item (Join-Path $app 'node_modules') (Join-Path $res 'node_modules') -Recurse

# 4. payload-1.1.0: the same, with a deterministic set of changes.
$v2 = Join-Path $Work 'payload-1.1.0'
if (Test-Path $v2) { Remove-Item $v2 -Recurse -Force }
Copy-Item $v1 $v2 -Recurse
$random = New-Object System.Random 20260101
$js = @(Get-ChildItem (Join-Path $v2 'resources\app\node_modules') -Recurse -File -Filter '*.js')
$changed = 0
foreach ($file in ($js | Sort-Object FullName | Get-Random -Count 300 -SetSeed 7)) {
    Add-Content -LiteralPath $file.FullName -Value "`n// 1.1.0`n" -Encoding ASCII
    $changed++
}
Add-Content -LiteralPath (Join-Path $v2 'resources\app\main.js') -Value "`n// 1.1.0`n" -Encoding ASCII
# A large binary changes too, the way a runtime library does in a real release.
$blob = Join-Path $v2 'snapshot_blob.bin'
$bytes = New-Object byte[] 4096
$random.NextBytes($bytes)
$stream = [IO.File]::Open($blob, 'Append')
$stream.Write($bytes, 0, $bytes.Length)
$stream.Close()
# Files that are new in 1.1.0, and files that are gone.
$extra = Join-Path $v2 'resources\app\new-in-1.1.0'
New-Item -ItemType Directory -Force $extra | Out-Null
1..60 | ForEach-Object { Set-Content -Encoding ASCII (Join-Path $extra "module-$_.js") "module.exports = $_;`n" }
$gone = @(Get-ChildItem (Join-Path $v2 'resources\app\node_modules\lodash') -File -Filter '*.js' | Sort-Object Name | Select-Object -First 40)
$gone | Remove-Item -Force

# 5. payload-tiny: one small program, for the fixed cost of an installer.
$tiny = Join-Path $Work 'payload-tiny'
if (Test-Path $tiny) { Remove-Item $tiny -Recurse -Force }
New-Item -ItemType Directory -Force $tiny | Out-Null
if (-not $TinyProgram) { $TinyProgram = Join-Path $env:SystemRoot 'System32\where.exe' }
Copy-Item $TinyProgram (Join-Path $tiny 'BenchApp.exe')

foreach ($name in 'payload-1.0.0', 'payload-1.1.0', 'payload-tiny') {
    $files = Get-ChildItem (Join-Path $Work $name) -Recurse -File
    '{0,-15} {1,6} files {2,8:N1} MB' -f $name, $files.Count, (($files | Measure-Object Length -Sum).Sum / 1MB)
}
