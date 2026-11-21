# Installer comparison: what the numbers say

One real Electron app (355 MB, 10,107 files), four tools, one computer, per-user
silent installs. The tables are in [RESULTS.md](RESULTS.md); how they were made,
and what they cannot tell you, is in [README.md](README.md). Run on 2026-11-21.

## Short version

- **Updating is where Bach is far ahead.** 1.0.0 to 1.1.0 took 2.4 s with Bach,
  12.5 s with NSIS, 24.1 s with Inno Setup and 135.9 s with Windows Installer.
  Bach only writes the files that changed, and removes the ones the new version
  no longer has. NSIS and Inno wrote everything again and left 40 old files
  behind; the MSI removed them but took more than two minutes.
- **A first install is a tie, not a win.** NSIS (13.7 s, zlib, the biggest
  file) was fastest; Bach (18.2 s), Inno Setup (20.8 s) and WiX (20.8 s) are
  within 15 % of each other. Writing 10,000 files dominates.
- **Bach's default build is the slowest to make and not the smallest.** 120.9 s
  and 106.9 MB, against 68.9 s and 99.6 MB for Inno's default. Bach's `--fast`
  build takes 2.8 s but is 134 MB. Inno (87.9 MB) and NSIS (90 MB) with their
  heaviest settings are smaller than anything Bach builds.
- **An installer costs more with Bach on a tiny app.** The runtime that draws the
  window is about 9.4 MB; Inno adds 1.9 MB, NSIS and MSI add nothing measurable
  (the small program compresses better than it was). If your app is 2 MB, that
  matters.
- **Failures are where the designs differ.** With a file locked during an
  update, only Bach and the MSI left the old version intact; Inno and NSIS left
  a mix of old and new files. After a kill half way through, Bach and NSIS
  recovered completely on the second run; Inno left a stray temporary file and
  a second uninstaller (`unins001.exe`).

## Speed

| | Bach | Inno Setup | NSIS | WiX (MSI) |
| --- | ---: | ---: | ---: | ---: |
| Install | 18.2 s | 20.8 s | **13.7 s** | 20.8 s |
| Update | **2.4 s** | 24.1 s | 12.5 s | 135.9 s |
| Remove | 6.8 s | **2.4 s** | 2.8 s | 33.3 s |

Bach's update is fast because setup skips files that already match the new
version (`work/install.rs` counts them as unchanged). The Inno and NSIS scripts
here copy every file again. The MSI's time is Windows Installer removing the old product and installing
the new one as a major upgrade, which is what its default does.

Bach's removal is slower than Inno's and NSIS's. The uninstaller hands over to a
copy of itself so that it can delete its own file; the benchmark counts the time
until the folder and the Settings entry are gone. Where the rest of the 6.8 s
goes was not profiled.

## Size

| Tool | Default | Heaviest compression | Time for the heavy build |
| --- | ---: | ---: | ---: |
| Bach | 106.9 MB (121 s) | 134.4 MB with `--fast` (2.8 s) | |
| Inno Setup | 99.6 MB (69 s) | 87.9 MB | 96.8 s |
| NSIS | 129.9 MB (35 s) | 90 MB | 135.7 s |
| WiX (MSI) | 110.4 MB (58 s) | 102.8 MB | 82.4 s |

Bach has no "slower, smaller" setting at the moment. Its default is its best
compression. Build memory is high (882 MB) for the same reason.

## Safety

| | Bach | Inno Setup | NSIS | WiX (MSI) |
| --- | --- | --- | --- | --- |
| File locked during an update | old version intact | mixed old and new | mixed old and new, exit 0 | old version intact |
| One flipped byte in the installer | refused, nothing written | refused, nothing written | refused, nothing written | refused, nothing written |
| Killed half way, then run again | correct | stray temp file, second uninstaller | correct | not tested |
| User files in the install folder after removal | kept | kept | **deleted** | kept |
| Old files after an update | removed | **40 left** | **40 left** | removed |
| Left in the profile after removal | nothing | nothing | nothing | nothing |

Things to know before you read the table as a verdict:

- The Inno and NSIS scripts here are the plain ones: no `[InstallDelete]`, no
  `Delete`s of the removed files, `RMDir /r` in NSIS. A careful script author
  fixes the stale files and the user-data deletion. What Bach does by default,
  these tools need a script for; that is the point of the comparison, and it
  is also why the table is not about what is *possible* with the other tools.
- NSIS reported exit 0 on the locked-file update although it did not finish. A
  script that checks for the file and aborts would behave better.
- The MSI's "old version intact" comes from Windows Installer's transaction
  rollback. Its price is the 136 s update and the 33 s removal.
- The MSI is installed per user, but Windows still records it under the
  machine-wide uninstall key. The run needed extra care to keep one measurement
  from contaminating the next (see the notes in `run.ps1`).

## What this does not show

- One computer, one app, Defender on. Another app (many small files, or a few
  huge ones) moves the numbers.
- Features were not scored: Inno and NSIS have far more scripting power than
  Bach; the MSI is what IT departments deploy with Group Policy and Intune;
  Bach has the built-in updater, one config file and a window that looks like
  a 2026 app. Pick by what you need.
- Bach's own numbers come from this repository's release build at the time of
  the run; they will change as it does.
