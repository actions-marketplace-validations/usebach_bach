# Writes the winget manifests for built setup files and lets winget itself
# validate them, so a manifest the Windows Package Manager would refuse fails
# here, not in a pull request to microsoft/winget-pkgs.
#
#   powershell -ExecutionPolicy Bypass -File scripts\test-winget.ps1 [-Profile release]
#
# It needs winget (GitHub's windows runners have it), the example installer
# (scripts\build-example.ps1) and bach.exe. It checks the example, and an app
# whose id is 50 characters long, and whose publisher is long as well: each
# part of a winget package id may have 32.

param([string]$Profile = 'release')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$bach = Join-Path $root "target\$Profile\bach.exe"
$example = Join-Path $root 'dist\Hello-Setup-1.0.0.exe'
foreach ($file in $bach, $example) { if (-not (Test-Path $file)) { throw "Build first: $file is missing." } }
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) { throw 'winget is not installed on this computer, so the manifests cannot be validated.' }

$work = Join-Path ([IO.Path]::GetTempPath()) ("bach-winget-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null
$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "  ok    $what" } else { Write-Host "  FAIL  $what" -ForegroundColor Red; $script:failures++ }
}
function Validate([string]$name, [string]$setup) {
    Write-Host $name
    $out = Join-Path $work ($name -replace '\W', '_')
    & $bach winget $setup --url 'https://example.com/Setup.exe?x=1#frag' --license MIT --out $out | Out-Host
    Check ($LASTEXITCODE -eq 0) 'bach winget writes the manifests'
    $text = (winget validate --manifest $out 2>&1 | Out-String)
    Check ($LASTEXITCODE -eq 0) "winget validate accepts them ($($text.Trim()))"
    return $out
}

try {
    $folder = Validate 'The example installer' $example
    $installer = Get-Content (Join-Path $folder '*.installer.yaml') -Raw
    Check ($installer -match 'InstallerUrl: "https://example.com/Setup.exe\?x=1#frag"') 'the link is quoted, so # and ? stay part of it'

    # An app with a 50-character id and a long publisher.
    $project = Join-Path $work 'longid'
    New-Item -ItemType Directory -Path "$project\app" | Out-Null
    Copy-Item (Join-Path $root 'target\release\hello.exe') "$project\app\hello.exe"
    $id = 'BachWingetValidationApp' + ('x' * 27)
    @"
[app]
name = "Long"
id = "$id"
version = "1.0.0"
publisher = "A Publisher With A Rather Long Name Incorporated Worldwide"
executable = "hello.exe"
[files]
source = "app"
"@ | Set-Content "$project\bach.toml"
    & $bach build "$project\bach.toml" --fast --out "$project\Long-Setup.exe" | Out-Null
    Check ($LASTEXITCODE -eq 0) 'builds an app with a 50-character id'
    $folder = Validate 'An app with a 50-character id' "$project\Long-Setup.exe"
    $package = (Get-Content (Join-Path $folder '*.yaml' ) -Raw | Select-String 'PackageIdentifier: (\S+)').Matches[0].Groups[1].Value
    $parts = $package.Split('.')
    Check ($parts.Count -eq 2 -and $parts[0].Length -le 32 -and $parts[1].Length -le 32) "each part of $package has at most 32 characters"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
if ($failures -ne 0) { Write-Host "$failures check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'All winget checks passed.'
