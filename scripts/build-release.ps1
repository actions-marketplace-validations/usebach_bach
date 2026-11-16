# Builds bach.exe and bach-setup.exe in release mode and packs them into
# dist\bach-<version>-windows-x64.zip, the download people use.
#
#   powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    # bach has the runtime's SHA-256 built in, so it can tell a real
    # bach-setup.exe from a swapped download: build the runtime first.
    cargo build --release -p bach-setup
    if ($LASTEXITCODE -ne 0) { throw 'The runtime build failed.' }
    $env:BACH_RUNTIME_SHA256 = (Get-FileHash -Algorithm SHA256 'target\release\bach-setup.exe').Hash.ToLower()
    cargo build --release -p bach-cli -p bach-setup
    if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }
    if ((Get-FileHash -Algorithm SHA256 'target\release\bach-setup.exe').Hash.ToLower() -ne $env:BACH_RUNTIME_SHA256) {
        throw 'bach-setup.exe changed between the two builds, so the hash built into bach would be wrong.'
    }

    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object { $_.name -eq 'bach-cli' }).version

    $stage = Join-Path 'dist' "bach-$version"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item 'target\release\bach.exe', 'target\release\bach-setup.exe', 'LICENSE' -Destination $stage
    # The bare runtime the release publishes is this exact file.
    Copy-Item 'target\release\bach-setup.exe' -Destination 'dist\bach-setup.exe'
    Copy-Item 'docs\getting-started.md' -Destination (Join-Path $stage 'README.md')

    $zip = Join-Path 'dist' "bach-$version-windows-x64.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    Remove-Item -Recurse -Force $stage

    $size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "Wrote $zip ($size MB)"
}
finally {
    Pop-Location
}
