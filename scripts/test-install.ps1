# Runs install.ps1 against a release made on the spot: a zip of the bach and
# bach-setup you give it, with its SHA-256, in a folder that stands in for
# GitHub.
#
#   powershell -ExecutionPolicy Bypass -File scripts\test-install.ps1 [-Profile debug|release] [-TouchPath]
#
# It checks the first install, a second run, the refusals (a damaged zip, a
# wrong checksum, a missing download, a folder that isn't Bach's) and the
# files that end up in place. Everything happens in a temporary folder and
# nothing is downloaded. The user's PATH is left alone unless -TouchPath is
# given (CI does, on a machine that is thrown away): then the test also checks
# that the folder is added once and takes it out again afterwards.

param([string]$Profile = 'debug', [switch]$TouchPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
foreach ($file in 'bach.exe', 'bach-setup.exe') {
    if (-not (Test-Path (Join-Path $root "target\$Profile\$file"))) { throw "Build first: target\$Profile\$file is missing." }
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("bach-install-test-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null
$failures = 0
function Check([bool]$ok, [string]$what) {
    if ($ok) { Write-Host "  ok    $what" } else { Write-Host "  FAIL  $what" -ForegroundColor Red; $script:failures++ }
}

$version = '9.9.9'
$name = "bach-$version-windows-x64"
function Make-Release([string]$dir, [switch]$Corrupt, [switch]$NotAZip) {
    New-Item -ItemType Directory -Path $dir | Out-Null
    $zip = Join-Path $dir "$name.zip"
    if ($NotAZip) {
        Set-Content -Path $zip -Value 'not a zip'
    } else {
        $pack = Join-Path $work "pack-$([Guid]::NewGuid().ToString('N').Substring(0, 6))"
        New-Item -ItemType Directory -Path $pack | Out-Null
        Copy-Item (Join-Path $root "target\$Profile\bach.exe"), (Join-Path $root "target\$Profile\bach-setup.exe") $pack
        Compress-Archive -Path "$pack\*" -DestinationPath $zip
    }
    $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    Set-Content -NoNewline -Path "$zip.sha256" -Value "$hash  $name.zip"
    if ($Corrupt) {
        $bytes = [IO.File]::ReadAllBytes($zip)
        $bytes[[int]($bytes.Length / 2)] = $bytes[[int]($bytes.Length / 2)] -bxor 0xFF
        [IO.File]::WriteAllBytes($zip, $bytes)
    }
}

# install.ps1 runs in a child PowerShell with the variables it reads, so the
# test's own session is never changed.
function Install([string]$from, [string]$target, [hashtable]$extra = @{}) {
    $script = Join-Path $root 'install.ps1'
    $variables = @{ BACH_VERSION = $version; BACH_INSTALL_FROM = $from; BACH_INSTALL_DIR = $target; BACH_SCOPE = 'user' }
    if (-not $TouchPath) { $variables['BACH_NO_PATH'] = '1' }
    foreach ($key in $extra.Keys) { $variables[$key] = $extra[$key] }
    $lines = @($variables.GetEnumerator() | ForEach-Object { if ($null -ne $_.Value) { "`$env:$($_.Key) = '$($_.Value)'" } else { "Remove-Item Env:$($_.Key) -ErrorAction SilentlyContinue" } })
    # An error ends the child with its message on stdout and exit code 1.
    $command = ($lines -join '; ') + "; try { & '$script' } catch { Write-Output `$_.Exception.Message; exit 1 }; exit 0"
    # The script's error messages come on stderr; that is output here, not a failure.
    $ErrorActionPreference = 'Continue'
    $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -Command $command 2>&1 | Out-String
    return [pscustomobject]@{ Exit = $LASTEXITCODE; Text = $output }
}
function User-Path() { [Environment]::GetEnvironmentVariable('Path', 'User') }

try {
    Make-Release (Join-Path $work 'release')
    $bin = Join-Path $work 'bin'

    Write-Host 'First install'
    $pathBefore = User-Path
    $run = Install (Join-Path $work 'release') $bin
    Check ($run.Exit -eq 0 -and (Test-Path (Join-Path $bin 'bach.exe')) -and (Test-Path (Join-Path $bin 'bach-setup.exe'))) "installs bach.exe and bach-setup.exe ($($run.Text.Trim()))"
    Check (Test-Path (Join-Path $bin '.bach-cli')) 'the folder is marked as Bach''s'
    $reported = & (Join-Path $bin 'bach.exe') --version 2>&1 | Out-String
    Check ($reported -match 'bach') "the installed bach runs ($($reported.Trim()))"
    if ($TouchPath) {
        $count = @((User-Path) -split ';' | Where-Object { $_.TrimEnd('\') -ieq $bin }).Count
        Check ($count -eq 1) 'the folder is on the user PATH, once'
    } else {
        Check ((User-Path) -eq $pathBefore) 'the user PATH is untouched (BACH_NO_PATH)'
    }

    Write-Host 'A second run changes nothing'
    $first = (Get-FileHash -Algorithm SHA256 (Join-Path $bin 'bach.exe')).Hash
    $run = Install (Join-Path $work 'release') $bin
    Check ($run.Exit -eq 0 -and (Test-Path (Join-Path $bin 'bach.exe'))) 'runs again without an error'
    Check ($first -eq (Get-FileHash -Algorithm SHA256 (Join-Path $bin 'bach.exe')).Hash) 'the same bach.exe is there'
    if ($TouchPath) {
        $count = @((User-Path) -split ';' | Where-Object { $_.TrimEnd('\') -ieq $bin }).Count
        Check ($count -eq 1) 'and the PATH still has the folder only once'
    }

    Write-Host 'A folder that is not Bach''s stays as it is'
    $foreign = Join-Path $work 'foreign'
    New-Item -ItemType Directory -Path $foreign | Out-Null
    Set-Content -Path (Join-Path $foreign 'notes.txt') -Value 'mine'
    $run = Install (Join-Path $work 'release') $foreign
    Check (-not (Test-Path (Join-Path $foreign 'bach.exe')) -and $run.Exit -eq 1) 'nothing is installed there and it fails'
    Check ($run.Text -match "aren't from this script") "and it says why ($($run.Text.Trim()))"

    Write-Host 'A damaged zip'
    Make-Release (Join-Path $work 'damaged') -Corrupt
    $run = Install (Join-Path $work 'damaged') (Join-Path $work 'bin2')
    Check (-not (Test-Path (Join-Path $work 'bin2\bach.exe'))) 'installs nothing'
    Check ($run.Text -match "doesn't match its published SHA-256") "and says so in a sentence ($($run.Text.Trim()))"

    Write-Host 'A checksum that fits a file that is not a zip'
    Make-Release (Join-Path $work 'notzip') -NotAZip
    $run = Install (Join-Path $work 'notzip') (Join-Path $work 'bin3')
    Check (-not (Test-Path (Join-Path $work 'bin3\bach.exe'))) 'installs nothing'
    Check ($run.Text -match "isn't a readable archive") "and says so ($($run.Text.Trim()))"

    Write-Host 'A missing download'
    $empty = Join-Path $work 'empty'
    New-Item -ItemType Directory -Path $empty | Out-Null
    $run = Install $empty (Join-Path $work 'bin4')
    Check (-not (Test-Path (Join-Path $work 'bin4\bach.exe'))) 'installs nothing'
    Check ($run.Text -match 'has no Windows download') "and says so ($($run.Text.Trim()))"
    $run = Install (Join-Path $work 'nowhere') (Join-Path $work 'bin5')
    Check ($run.Text -match "isn't a folder") 'a folder that is not there is named'
    $run = Install (Join-Path $work 'release') (Join-Path $work 'bin6') @{ BACH_VERSION = $null }
    Check (-not (Test-Path (Join-Path $work 'bin6\bach.exe')) -and $run.Text -match 'needs BACH_VERSION') 'a local install without a version is refused'
} finally {
    if ($TouchPath) {
        # Take the folder out of the PATH again: this test was never the user's install.
        $bin = Join-Path $work 'bin'
        $kept = ((User-Path) -split ';' | Where-Object { $_ -ne '' -and $_.TrimEnd('\') -ine $bin }) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $kept, 'User')
    }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures -ne 0) { Write-Host "$failures check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'All install.ps1 checks passed.'
# The last checks run child processes that fail on purpose; without this the step ends with
# their exit code (CI's pwsh passes $LASTEXITCODE on) although every check passed.
exit 0
