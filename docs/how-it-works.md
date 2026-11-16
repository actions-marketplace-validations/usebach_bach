# How Bach works

## One executable, three parts

`bach-setup.exe` is the installer window. On its own it installs nothing.
`bach build` copies it, gives the copy your icon and version details, and
appends your app to its end:

```
[bach-setup.exe] [archive] [logo] [manifest] [padding] [trailer]
```

- **archive**: every file from `[files] source` in one zstd stream.
- **logo**: the bytes of `[installer] logo` (or the PNG inside `app.icon`), if any.
- **manifest**: JSON describing the app: name, version, publisher, the files
  and their sizes, the options, components, texts and license, the look.

The build reads every file once. Each file's SHA-256 is taken from the same
bytes that go into the archive, and the manifest, which sits behind the archive,
is written last with those checksums in it. A build of unchanged files gives
the same setup file, byte for byte.
- **trailer**: 40 bytes: `BACHPACK`, the format version, the padding length
  and the length of each section.

Windows ignores bytes after an executable's last section, so the setup file
still runs as `bach-setup.exe`. On start it reads its own trailer, finds the
manifest, and shows the install window. `crates/pack/src/container.rs` has
the exact layout. Fields added to the manifest over time all have defaults,
so a newer setup file still reads the uninstaller an older one installed.

## Installing

1. Checks before anything changes: what the app needs (requirements) is
   there, no other setup for the same app is running, the app isn't open,
   and the drive has room for every file.
2. Every chosen file is unpacked next to its target as `<name>.bach-new`.
   The setup file is read once: while it is unpacked, its archive is checked
   against the SHA-256 recorded when it was built, so the progress bar moves
   from the first byte. If the archive doesn't match, everything unpacked is
   deleted and nothing is replaced. Only when all of them are on disk and the
   check has passed are the files swapped into place, each old
   file stepping aside as `<name>.bach-old` first. If one can't be moved (a
   file in use), every move is undone: an update either completes or leaves
   the earlier version exactly as it was.
3. If an earlier version is installed, the files it installed and this one
   doesn't (including components the user unticked) are removed. The earlier
   file list comes from its uninstaller.
4. `uninstall.exe` is written: `bach-setup.exe` again, with the manifest of
   what was installed and the user's choices, and no archive.
5. Shortcuts, start with Windows, the `PATH` entry, link schemes and file
   types are added (or taken away, when an update unticked them), and the
   app is listed under **Settings › Apps**
   (`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<id>`).

Everything is per user by default, so nothing asks for administrator rights.
An app that sets `installer.scope = "machine"` installs the same way into
`%ProgramFiles%` and `HKLM` instead, after setup has started itself again with
administrator rights (one Windows prompt, before anything is changed). The
window can't be closed while files are moving.

## The online installer

`bach build --online` also writes a setup file that is the same runtime with
the same manifest and no archive. When it runs it asks the update feed, downloads
the newest full setup file into `%LOCALAPPDATA%\Bach\updates\<app id>` (resuming
if it was cut off), checks it exactly like an update (SHA-256 against the
feed, same app id, the feed's version) and then installs from that file. The
rest is the normal install, with the downloaded runtime. The download is deleted
once the app is installed. Nothing in the online installer says which version
it will install, so it never shows the "older version" page.

## Access keys

An app with `[access]` carries encryption settings in its manifest (salt, nonce,
Argon2id costs and a hash that tells a right key from a wrong one), and its
archive is encrypted in authenticated chunks (see
[`[access]`](config.md#access)). Setup derives the key on a background thread,
checks it against the manifest before touching the disk, and decrypts while
unpacking, so the all-or-nothing install is unchanged. A manifest that asks for
more memory than 256 MiB or an unknown scheme is refused. The key kept for
updates is sealed with DPAPI, tied to the Windows user and the app's id.

## Updates from the app

With `[updates]`, the installed `uninstall.exe` (the runtime with the
app's manifest) answers `--check-update`, `--download-update`,
`--apply-update`, `--update` and `--auto-update`. It reads the feed's JSON
with Windows' own `curl.exe`, https only, and downloads the newer setup file
into `%LOCALAPPDATA%\Bach\updates\<app id>`, where it waits (and continues
after an interruption) until it is installed. Before it runs, it is hashed
again and must match the feed's SHA-256, carry the same app id and the
version the feed names. The setup file does the update as usual, silently
when it's started by `--apply-update` or the background task. See
[updates](updates.md).

The "Automatic updates" box registers a per-user scheduled task,
`Bach\<app id>`, that runs `uninstall.exe --auto-update` every few hours. It
installs only while the app isn't running. Unticking the box, an update
without it, and uninstalling all delete the task.

The running uninstaller can run an update: Windows lets a running program be
renamed but not overwritten, so the new `uninstall.exe` takes its place and
the old copy (`uninstall.exe.bach-old`) is deleted once the process ends.

## Interrupted updates

Moving the new files into place is all or nothing while the process lives. A
power cut or a crash leaves no process to undo anything, so before the first
move a small journal (`.bach-update`) lists every file that is about to
move. The next install, update or removal, and each background round while
the app is closed, reads it and puts back what was half moved. When all the
moves are done, the new files are flushed to disk, then the journal and the
old copies go. A journal can only act on paths inside the install folder.

## Logs

Every run writes `%TEMP%\<app id>-setup.log` (a space in the id is written `%20` there and in the staging and download folders, so `My App` and `My-App` never share one; or `--log FILE`): the steps
and, when something fails, why. Logging never holds setup up.

## Languages

The window's texts live in `crates/pack/src/i18n`, one table per language.
At start the window reads the user's Windows display languages, most
preferred first, and picks the first one the app offers
(`installer.languages`, or every built-in language), else the app's first
language. The developer's `[text]` replaces any built-in text, per language.

## Removing

Settings › Apps runs `uninstall.exe`. A running program can't delete its own
file, so it copies itself to the temp folder, starts the copy and exits. The
copy shows the window and removes:

- the program first (if the app is still open, nothing has changed yet),
- every file the manifest lists, and the folders they leave empty,
- the shortcuts and the Apps entry,
- start with Windows, the `PATH` entry, link schemes and file types, each
  only if it's this install's own,
- the data folders, only if the user ticked the box,
- the install folder, only if it's empty, so files a user kept there stay.

Then a hidden helper deletes the temp copy once it has exited. A silent
removal (`uninstall.exe --silent`) works in place instead, so whoever
started it can wait for its exit code.

## Safety rules

- Packaged paths are relative and can never contain `..`, so a setup file
  can't write outside its install folder.
- Data folders must sit inside the user's profile (`%APPDATA%`,
  `%LOCALAPPDATA%`, `%USERPROFILE%`) and name a folder of their own: never
  `Documents`, `Desktop`, `AppData`, anything in `.ssh` or `Microsoft`, and
  never the install folder or a folder that contains it.
- The uninstaller refuses a folder that doesn't hold the app, and an update
  only cleans up after an earlier install of the same app id.
- A machine-wide install reads the install folder's access list before it
  writes and before an elevated update or removal, and refuses a folder that
  ordinary users can change or that Administrators, SYSTEM or TrustedInstaller
  don't own.
- A first install into a folder that already holds files of the same names
  (and no earlier install of this app) warns in the window, naming them, and
  is refused by a silent run unless it gets `--overwrite`.
- The uninstaller keeps a file whose SHA-256 no longer matches the manifest,
  because someone changed it after setup wrote it, and says so in the log.
- The manifest is checked again when a setup file starts, so a damaged or
  edited file fails with a sentence instead of misbehaving.
- `PATH` is edited as raw text: exactly one entry is added or removed, the
  value's type (`REG_EXPAND_SZ`) is kept, and nothing is written when
  there's nothing to change.
- Registry keys Bach creates for link schemes and file types carry a
  `BachApp` mark; keys without it belong to another program and are never
  changed or deleted.
- Registry values, environment variables and right-click entries from
  `bach.toml` stay below `Software\<your key>` (never `Classes`, `Policies`,
  Windows' own keys or Bach's) and are removed only while they still hold what
  setup wrote. Right-click entries carry the `BachApp` mark like link schemes.
- Administrator rights are asked for once and only when the app says so
  (`installer.scope = "machine"`, or `elevated = true` on one run step).
  A machine-wide uninstaller removes itself in place instead of from a temp
  copy, and setup files link with `/DEPENDENTLOADFLAG` so only System32's DLLs
  load, never one planted next to a setup file.
- `[[migrate]]` runs only the uninstall command of a named Settings › Apps
  entry, never through a shell. An elevated setup ignores per-user entries,
  and a machine-wide old uninstaller runs elevated only from Program Files or
  the Windows folder.
- Signing runs a program from `bach.toml` only when you ask for it with
  `bach build --sign`.
- The C runtime is linked in, so setup files start on a fresh Windows
  without the Visual C++ redistributable.

## Run steps

`[[run]]` programs start only from the app's own packaged files, found through
`paths.rs`, with no shell, and are waited for (15 minutes at most). They run as
the user unless a step says `elevated = true`, which makes Windows ask for
administrator rights for that program alone. Steps that run before removal can
fail without stopping the removal.

## An update writes only what changed

The manifest records each file's size and SHA-256. Before unpacking, setup
checks the files already in the install folder against it, and a file that
matches is not written again, also when the setup file is a full one. That
shortens updates and means an unchanged program that is running is no obstacle.
The free-space check counts only what is written. Setup files built before
Bach recorded hashes write every file as before.

## Removal is all or nothing

The uninstaller first looks that the app is closed. Then it renames every file
the manifest lists to `<name>.bach-old`, which is quick and can be undone. If
one can't move (a locked DLL, security software, a cloud folder), the ones
already moved go back, the message names the file, and the app keeps working and
stays registered. Only when all have moved does it remove shortcuts and
integration; if that fails before the app is unregistered, the files go back
too. Then the Settings > Apps entry goes, and the renamed files are deleted. A
file whose content changed after setup wrote it is not touched (see above), and
a locked file counts as unchanged, so it stops the removal instead of being
kept silently.

## When a step after the files fails

An update (or first install) puts its files in place first, then runs the
`[[run]]` steps, shortcuts, `PATH`, link schemes and file types, writes the new
`uninstall.exe` and registers the version. The old copies of the files and the
journal stay until the last of those has worked. If one fails, the files go back
to the earlier version, the earlier `uninstall.exe` is restored, shortcuts and
integration are written again from the earlier manifest, and the version under
Settings > Apps never changed. Files the earlier version had and this one drops
are removed only after that. A first install that fails this way leaves no files
behind. What a run step already did on its own (a database it set up, say)
can't be undone by setup. A power cut between the file swap and the end is
rolled back by the journal the next time setup runs; the earlier
`uninstall.exe` is restored only when setup itself sees the failure.

## Where the runtime comes from

Every setup file contains the runtime, so `bach` is strict about it:

- A released `bach` has the SHA-256 of its version's `bach-setup.exe` built in
  (the release workflow sets `BACH_RUNTIME_SHA256` while building it). A
  download, and a copy in the cache, must match it. A `bach` built from source
  has none and checks a download against the `.sha256` published next to it,
  which only guards against a damaged transfer. `bach doctor` shows the
  runtime's SHA-256 and which of the two applies.
- Downloads use https only, redirects included. `BACH_RELEASES`, which points
  at a mirror of the release downloads, must start with `https://`.
- The cache is `%LOCALAPPDATA%\bach\runtime\<version>` on Windows and
  `$XDG_CACHE_HOME/bach/runtime/<version>` (or `~/.cache/bach/...`) elsewhere.
  It is created for its owner only, and a cached runtime is hashed again
  before every use. Without `LOCALAPPDATA`, or `HOME` and `XDG_CACHE_HOME`,
  there is no cache and no fallback to the shared temp folder: `bach build`
  says so and asks for `--runtime` or `BACH_RUNTIME`.
- `BACH_NO_DOWNLOAD` forbids the download altogether.

## Windows on ARM

A setup file is the setup runtime with the app's files appended, and the
runtime is a native Windows program. The `bach-setup.exe` that `bach build`
downloads is built for x64, which Windows 11 on ARM runs through its emulation.
For a native ARM64 setup file, build with the ARM64 runtime: every Bach release
has a `bach-<version>-windows-arm64.zip` with an ARM64 `bach.exe` and
`bach-setup.exe`.

```bash
bach build --runtime path/to/arm64/bach-setup.exe
```

(`BACH_RUNTIME` does the same.) The runtime copies files and writes registry
values, so it doesn't care what architecture your app is; which one to ship is
your choice. Requirements that look for a runtime are architecture-specific:
`kind = "vcredist"` looks for the x64 redistributable.

## Signing

The payload sits before the signature's certificate table, so `signtool`
signs a setup file like any other program and the signature covers the
payload. See [signing](signing.md).
