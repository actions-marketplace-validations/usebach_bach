# Installer comparison

A repeatable comparison of Bach with Inno Setup, NSIS and WiX on one real app.
The numbers are in [RESULTS.md](RESULTS.md), what they mean is in
[FINDINGS.md](FINDINGS.md), and this page says how they were made and what they
do not say.

## The app

A real Electron application: the Electron 33.2.0 runtime for Windows plus the
production `node_modules` of seven well-known packages (lodash, moment, rxjs,
three, typescript, webpack, @babel/core). 10,107 files, 355 MB. It is built by
[make-payload.ps1](make-payload.ps1), which also makes:

- `payload-1.1.0`: the next release. About 300 JavaScript files changed, a large
  binary grew by 4 KB, 60 files are new and 40 are gone. This is what an update
  has to cope with.
- `payload-tiny`: one small program, to measure what an installer adds by itself.

## What is compared

Every tool builds the same payload and installs it **per user, without
administrator rights**, into a folder under the work directory, silently.
No tool shows a prompt, and none needs elevation.

| Tool | Version | Script or config | Silent install | Settings of the two builds |
| --- | --- | --- | --- | --- |
| Bach | this checkout, release build | `bach.toml` | `--silent --dir` | the defaults; `--fast` |
| Inno Setup | 7.1.1 | `.iss` | `/VERYSILENT /DIR=` | the defaults (lzma2/max, not solid); `lzma2/ultra64` with solid compression |
| NSIS | 3.13 | `.nsi` | `/S /D=` | the default compressor (zlib, not solid); `SetCompressor /SOLID lzma` |
| WiX | 7.0.1 (MSI) | `.wxs` | `msiexec /i /qn` | the default cabinet; `CompressionLevel="high"` |

The scripts are in [run.ps1](run.ps1), so every setting is visible. They are
what a developer would write for this app in an afternoon: no shortcuts, no
custom pages, one component. NSIS removes the install folder with
`RMDir /r`, the idiom its own examples use.

## What is measured

- **Build**: wall-clock time and the peak working set of the build program.
- **Install, update, removal**: wall-clock time, three install and removal
  cycles for the median. After the first install the folder is compared with the
  payload: every path and size, and the SHA-256 of 400 files chosen with a fixed seed.
- **Update**: 1.0.0 installed, then 1.1.0 over it. The folder is compared with
  the 1.1.0 payload, and files that are only in 1.0.0 are counted as stale.
- **A locked file during an update**: `resources\app\main.js`, which the
  update changes, is held open without sharing while the update runs. The result
  is classified from the files the update changes: old version intact, new
  version complete, or a mix. A run that hangs is ended after 150 seconds.
- **A damaged installer**: one byte in the middle is flipped. Did the installer
  refuse, and did it write anything first?
- **Killed half way**: the install process is killed once 40 % of the files
  are there, then run again. Is the second run correct? (Not for MSI: Windows
  Installer's service does the work, so the client can't be killed meaningfully.)
- **User files**: a file and a folder the user added inside the install folder.
  Are they still there after removal?
- **Leftovers**: new folders and keys in the user's profile after a removal.

## What it does not say

- It is one computer (Intel Core i5-12400F, 15.7 GB of RAM, an SSD, Windows 11
  Home, Windows Defender on) and one app. A virus scanner reading every new
  file changes install times for every tool alike. Run it again on your own
  machine; the scripts are here for that.
- Install times are dominated by writing 10,000 files. The tools differ
  where they decompress and where they verify.
- A setup file written another way (a different script, a custom compressor
  setting, a 7-Zip archive inside an NSIS installer) would give other numbers.
  The scripts here are the plain way.
- Features that were not measured (languages, an updater, signing, rollouts)
  are listed in the docs, not scored here.

## Running it

```powershell
# Once: Inno Setup, NSIS and WiX (scoop), .NET (for WiX), bun and 7-Zip.
scoop install extras/inno-setup extras/nsis main/wixtoolset main/dotnet-sdk 7zip
wix eula accept wix7      # WiX asks for the OSMF EULA

cargo build --release -p bach-cli -p bach-setup -p hello
powershell -ExecutionPolicy Bypass -File bench\make-payload.ps1 -Work D:\bench-work
powershell -ExecutionPolicy Bypass -File bench\run.ps1 -Work D:\bench-work
powershell -ExecutionPolicy Bypass -File bench\summarize.ps1
```

`run.ps1 -Tools bach,nsis` runs a subset, `-Repeats 1` is quicker. It
installs only into `<Work>\install\<tool>` and removes the Settings › Apps
entries named BenchApp that it made. Don't run it on a computer that has an
app with that name installed.
