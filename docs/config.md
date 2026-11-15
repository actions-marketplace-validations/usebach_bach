# bach.toml reference

Every setting Bach reads. Paths are relative to the `bach.toml` file. Unknown
settings are an error, so a typo never goes unnoticed, and every mistake is
reported as one sentence naming the setting at fault.

Only `[app]` and `[files]` are required. Everything else has a sensible
default, so a five-line config already builds a complete installer.

- [`[app]`](#app): name, version, publisher, the program
- [`[files]`](#files): what gets installed
- [`[installer]`](#installer): look, languages, license and options
- [`[[components]]`](#components): optional parts the user can leave out
- [`[[protocols]]`](#protocols): links like `myapp://` that open your app
- [`[[file_types]]`](#file_types): files your app opens
- [`[[requirements]]`](#requirements): what the app needs on the computer first
- [`[[shortcuts]]`](#shortcuts): more Start menu shortcuts
- [`[[registry]]`](#registry), [`[[env]]`](#env) and [`[[context_menu]]`](#context_menu): registry values, environment variables and right-click entries
- [`[[migrate]]`](#migrate): remove an older install made by another installer
- [`[updates]`](#updates): updates the installed app fetches itself
- [`[text]`](#text): any text in the window, in any language
- [`[uninstall]`](#uninstall): the user's data
- [`[signing]`](#signing): sign the setup files while building (optional)
- [A complete example](#a-complete-example)
- [Silent installs](#silent-installs) and [updating from the app](#updating-from-the-app)
- [Editor support](#editor-support) and [listing your app in winget](#listing-your-app-in-winget)

## [app]

| Setting | Required | Meaning |
| --- | --- | --- |
| `name` | yes | The app's name, as people see it: the window, the Start menu, the desktop shortcut and Settings › Apps. |
| `version` | yes | Any version text, like `1.2.0` or `2.0.0-beta.1`. Shown in the window and Settings › Apps; the first numbers also go into the setup file's details. |
| `publisher` | yes | You or your company. Shown in the window's footer and Settings › Apps. |
| `executable` | yes | The program to start, relative to `[files] source`, for example `my-app.exe` or `bin/my-app.exe`. Shortcuts and the "Open" button start it. |
| `id` | no | Identifies the install across versions (registry key name). Defaults to `name`. Letters, digits, spaces, `.`, `-` and `_`. Never change it after your first release, or updates install side by side. |
| `description` | no | One line. The shortcut's tooltip, and the line under "Install My App" on the first page (unless `[text] install_subtitle` replaces it). Without it, that line is left out. |
| `website` | no | An `https://` link, shown as the support link in Settings › Apps. |
| `icon` | no | A `.ico` file. It becomes the setup file's icon and the window's icon, and, when it holds a PNG image (most modern icons do) and `installer.logo` isn't set, the logo in the window. Include sizes 16 to 256 for crisp results. |

## [files]

| Setting | Required | Meaning |
| --- | --- | --- |
| `source` | yes | The folder whose contents are installed, subfolders included. |
| `exclude` | no | Patterns to leave out. `*` matches within a name, `**` across folders, `?` one character. A pattern without `/` matches file names anywhere (`*.pdb`); one with `/` matches from the top of `source` (`docs/**`). Case doesn't matter. |

The source folder may not contain symbolic links or a file named
`uninstall.exe` at its top (Bach adds its own).

## [installer]

### Look

| Setting | Default | Meaning |
| --- | --- | --- |
| `output` | `dist/{name}-Setup-{version}.exe` | Where the setup file goes. `{name}`, `{version}` and `{id}` are filled in. |
| `theme` | `system` | `system` follows Windows' light or dark setting, live. `light` or `dark` fixes it. |
| `accent` | none (black / white) | A colour like `#2563eb` for buttons, the progress bar and ticked boxes. The button text picks black or white for contrast. |
| `logo` | your icon, else the Bach mark | An `.svg` or `.png` shown at the top of the window, or `"none"` for no logo at all. An SVG is drawn in one colour that follows the theme; use a PNG for a full-colour logo. A wide PNG (a wordmark) keeps its shape. Give a PNG twice the size it's shown at for sharp results. |
| `logo_height` | `56` | The logo's height in points, 32 to 96. |
| `online` | `false` | `true` also builds the **online installer** (see below). Needs [`[updates]`](#updates). `bach build --online` does the same once. |
| `online_output` | `dist/{name}-Web-Setup.exe` | Where the online installer goes. No version in the name, so one download link keeps pointing at the newest. |
| `compression` | `best` | `best` makes the smallest file, `normal` and `fast` build quicker. Unpacking is fast either way. `bach build --fast` overrides it once. |

### Online and offline installers

`bach build` writes the **offline installer**: one setup file that carries
the app, so it installs without a network and always installs the version it
was built from. With `online = true` (or `bach build --online`) it also writes
the **online installer**, a small setup file with the same window and
choices that carries no files. When it runs it reads the update feed, downloads
the *newest* full setup file, checks it (SHA-256 against the feed, same app
id, the version the feed names) and installs from that. It installs whatever
is current, not what was current when it was built, so you publish it once.

```toml
[installer]
online = true

[updates]
github = "owner/repo"
```

```console
$ bach build
Wrote dist\Hello-Setup-1.4.0.exe (9.7 MB)
Wrote dist\Hello-Web-Setup.exe (8.9 MB), the online installer
```

The online installer needs a network and the feed; if either is missing it
says so and "Try again" resumes the download. Choices that a newer version
added (a new component that is ticked by default) are carried over. The
feed describes the full setup file, never the online one. The Bach action
signs and publishes both files.

### Where it installs

| Setting | Default | Meaning |
| --- | --- | --- |
| `folder` | the app id | The install folder's name under `%LOCALAPPDATA%\Programs`. When the user picks another place, this name is added to it. |
| `change_folder` | `true` | Whether the user may pick another folder ("Change", or a click on the Location row). Updates always stay where they are. |
| `open_button` | `true` | Whether the last page offers "Open My App". |
| `start_menu_folder` | none | A folder in the Start menu for the shortcuts, like your company's name. Removed when it's empty after uninstalling. |
| `website_button` | `false` | Whether the last page links to `app.website`. |
| `scope` | `"user"` | `"user"` installs for the person who runs setup and needs no administrator rights. `"machine"` installs for every user of the computer, into `%ProgramFiles%`, and Windows asks for administrator rights once. See [machine-wide installs](#machine-wide-installs). |

### Machine-wide installs

With `scope = "machine"`, setup starts again with administrator rights before
it does anything: Windows shows its prompt once (or asks for an administrator's
password), then the whole install runs elevated. Nothing is hidden from that
prompt, and a setup run from an elevated terminal or by a deployment tool
that already has the rights doesn't ask.

```toml
[installer]
scope = "machine"
```

| | Per user (`"user"`, default) | Machine-wide (`"machine"`) |
| --- | --- | --- |
| Install folder | `%LOCALAPPDATA%\Programs\<folder>` | `%ProgramFiles%\<folder>` |
| Settings › Apps entry | `HKCU` | `HKLM` |
| Start menu and desktop shortcuts | the user's | every user's |
| Start with Windows, link schemes, file types | `HKCU` | `HKLM` |
| `PATH` (`add_to_path`) | the user's | the computer's |
| `[[registry]]`, `[[env]]`, `[[context_menu]]` | `HKCU` | `HKLM` |
| Administrator prompt | never | once, at the start |

Things to know:

- Updates and the uninstaller ask for administrator rights too. `uninstall.exe`
  does it when Settings › Apps starts it; an update started by the app asks
  when its setup file starts.
- Background updates (`[updates] auto`) are off: an update has to write to
  Program Files, which a signed-in user can't. `auto` must be left out or
  `"never"`. The app can still use `--check-update`, `--download-update` and
  `--update` (see [updating from the app](#updating-from-the-app)); Windows
  asks when the update installs.
- `--silent` works the same way. Run from a non-elevated terminal it shows
  Windows' prompt; declining it exits with code 1. Deployment tools that run
  setup elevated or as SYSTEM see no prompt.
- "Also delete my data" deletes the data folders of the account that runs the
  uninstaller, because `%APPDATA%` and `%LOCALAPPDATA%` belong to one user.
- Don't change `scope` after your first release: an app installed per user and
  one installed machine-wide are two installs to Windows, so an update with
  the other scope installs next to the old one instead of replacing it.

### Options

Each option takes one of four values:

| Value | The install page | What happens |
| --- | --- | --- |
| `true` | shows a ticked box | done, unless the user unticks it |
| `false` | shows an unticked box | done only if the user ticks it |
| `"always"` | shows nothing | always done |
| `"never"` | shows nothing | never done |

| Setting | Default | What it does |
| --- | --- | --- |
| `desktop_shortcut` | `true` | A shortcut on the desktop. |
| `start_menu_shortcut` | `"always"` | A shortcut in the Start menu. |
| `start_with_windows` | `"never"` | Starts the app when the user signs in (`HKCU\…\Run`). |
| `add_to_path` | `"never"` | Adds the app's folder to the user's `PATH`, so command-line tools run from any terminal. Only that one entry is ever added or removed; the rest of `PATH` is kept byte for byte. |
| `path_folder` | the install folder | The folder, inside the install folder, that goes on `PATH`, like `"bin"`. |

The whole row is a click target: clicking anywhere on it ticks or unticks
its box. On an update, the boxes start the way the user left them last time,
and unticking one takes the shortcut, PATH entry or autostart away again.

### Languages and license

| Setting | Default | Meaning |
| --- | --- | --- |
| `languages` | every built-in language | The languages the window may speak, like `["en", "de"]`. The window follows the user's Windows display language when it's in this list, and otherwise uses the first one. With a single language, like `["de"]`, the window always speaks it. |
| `license` | none | A UTF-8 text file the user accepts before the install page, like `"LICENSE.txt"`, or one per language: `{ en = "LICENSE.txt", de = "LIZENZ.txt" }`. An update shows it again only when it changed. |

Built-in languages: `en` English, `de` Deutsch, `nl` Nederlands, `fr`
Français, `es` Español, `it` Italiano, `pt` Português, `pl` Polski, `ru`
Русский, `tr` Türkçe, `ja` 日本語, `ko` 한국어, `zh` 简体中文.

## [[components]]

Optional parts of your app, each shown as a box on the install page. Files
that no component claims are always installed.

```toml
[[components]]
id = "samples"                      # lowercase; also its name for --with
title = "Sample projects"           # or { en = "…", de = "…" }
description = "Examples to learn from."
files = ["samples/**"]              # patterns, like [files] exclude
default = true                      # ticked on a first install
```

A file belongs to the first component whose pattern matches it. A component
must match at least one file, and `app.executable` can't be in one. Unticking
a component on an update removes its files.

## [[protocols]]

Links like `myapp://open?file=x` start your app with the link as its first
argument. Registered for the user only.

```toml
[[protocols]]
scheme = "myapp"
description = "My App link"         # optional
```

Schemes that belong to Windows or browsers (`http`, `file`, `ms-…`, …) are
refused. If another program already owns the scheme, setup leaves it alone.

## [[file_types]]

Files your app opens. They're listed under "Open with", and your app becomes
the default only for types no other program opens yet, so installing your
app never takes files away from another one.

```toml
[[file_types]]
extension = ".mydoc"
description = "My App document"     # optional
```

Types that run code (`.exe`, `.bat`, `.ps1`, `.lnk`, …) are refused.

## [[requirements]]

Something your app needs on the computer before it runs. Setup only looks
(read-only, no administrator rights), and when it's missing shows a page
with a button to its download and a "Try again" that looks again. A silent
install stops with exit code 1 instead.

```toml
[[requirements]]
kind = "webview2"            # Tauri and WebView2 apps

[[requirements]]
kind = "dotnet"              # a .NET runtime
version = "8.0"
desktop = true               # the Windows Desktop runtime (WinForms, WPF)

[[requirements]]
kind = "vcredist"            # the Visual C++ 2015-2022 redistributable (x64)

[[requirements]]
kind = "file"                # any file; %VARIABLES% are filled in
path = '%SystemRoot%\System32\vcruntime140.dll'
name = "Visual C++ runtime"
url = "https://aka.ms/vs/17/release/vc_redist.x64.exe"

[[requirements]]
kind = "registry"            # any key under HKLM\ or HKCU\
key = 'HKLM\SOFTWARE\Vendor\Thing'
name = "Vendor Thing"
url = "https://vendor.example/download"

[[requirements]]
kind = "windows"             # this Windows build number or newer
build = 19041                # 10240 Windows 10, 17763 1809, 19041 2004, 22000 Windows 11
```

`webview2`, `dotnet`, `vcredist` and `windows` know their name and download
page (`windows` points at microsoft.com/windows); `file` and `registry` need
`name` and an `https` `url`. `windows` reads the build number from
`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.

## [[run]]

Programs setup runs for you: what copying files can't do, like a runtime's
own installer or a driver. The program is one of your packaged files, so add
it under `[files] source`.

```toml
[[run]]
name = "Visual C++ runtime"            # "Setting up Visual C++ runtime"; the file's name by default
file = "redist/vc_redist.x64.exe"      # a packaged .exe, relative to [files] source
arguments = "/quiet /norestart"        # the command line, passed as written
when = "first_install"                 # "install" (default), "first_install" or "uninstall"
success_codes = [0, 3010]              # exit codes that mean it worked; 0 always counts
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `file` | required | A packaged `.exe`. Setup never runs anything else, and never through a shell. |
| `name` | the file's name | What the progress label says: "Setting up {name}" (`[text] step_running`). Shown as written in every language. |
| `arguments` | none | The command line, passed as written. |
| `when` | `install` | `install`: after the files are in place, on a first install and on every update. `first_install`: not on updates. `uninstall`: before the files are removed. |
| `success_codes` | `[0]` | Exit codes that mean it worked (0 always counts). Any other code stops the install with "{name} couldn't be set up (exit code N)". |
| `ignore_errors` | `false` | Go on when it fails. Not allowed with `when = "uninstall"`, because removing never stops for a failed step. |
| `component` | the file's component | Only run when this optional component is installed. |

| `elevated` | `false` | Run this step with administrator rights: Windows asks once when it starts (not at all when setup is elevated already, as with `scope = "machine"`). Setup itself stays unelevated. Declining fails the step like any other failure. |

Steps run as the user, in the program's own folder, in the order listed.
A program that asks for administrator rights fails to start, which is
reported like any other failure, unless the step says `elevated = true`: that
is for things Windows only lets an administrator do, like the Visual C++
redistributable's own installer:

```toml
[[run]]
name = "Visual C++ runtime"
file = "redist/vc_redist.x64.exe"
arguments = "/quiet /norestart"
success_codes = [0, 1638, 3010]   # installed, newer already there, restart needed
when = "first_install"
elevated = true
```

An elevated step starts the program from the install folder. In a per-user
install that folder is writable by the user's own programs, so use `elevated`
for well-known installers (and sign them), not for anything else. A step that
runs longer than 15 minutes is ended. Each run is in the log. `bach inspect`
lists them as `Runs`.

## [[registry]]

Registry values setup writes for the user, for apps that read their install
folder or a setting from the registry.

```toml
[[registry]]
key = 'Software\Hello'          # below HKCU, starting with Software\
name = "InstallPath"            # leave out for the key's default value
type = "string"                 # "string" (default), "expand" or "dword"
value = '{app}'                 # {app} is the install folder

[[registry]]
key = 'Software\Hello'
name = "FirstRun"
type = "dword"
value = 1
```

Use single quotes for the key, so the backslashes stay as they are. An
update writes the values again and removes the ones the new version no longer
lists. Removing the app deletes a value only while it still holds what setup
wrote, so a setting the user or the app changed since stays, and keys that end
up empty go too. Keys that belong to Windows, other programs or Bach
(`Software\Classes`, `Software\Policies`, `Software\Microsoft\Windows…`,
`Software\Bach`) are refused; use `[[protocols]]`, `[[file_types]]` and
`[[context_menu]]` for what those are for.

## [[env]]

Environment variables of the user (`HKCU\Environment`). Programs started after
the install see them; ones already running don't.

```toml
[[env]]
name = "HELLO_HOME"
value = '{app}'                 # {app} is the install folder
```

A value with a `%VARIABLE%` in it is stored so Windows fills it in when it's
read. Removing the app removes a variable only while it still holds the value
setup set. Names Windows uses (`PATH`, `TEMP`, `USERPROFILE`, …) are refused;
`PATH` has its own setting, `add_to_path`.

## [[context_menu]]

Entries in Explorer's right-click menu that start your app with the clicked
file or folder.

```toml
[[context_menu]]
name = "Open with Hello"
on = ["files", ".md"]           # files, folders, background, ".ext", or a list
arguments = '"%1"'              # optional; "%1" is the clicked item

[[context_menu]]
name = "Open Hello here"
on = "background"               # the empty space inside an open folder
```

| `on` | Shows on |
| --- | --- |
| `"files"` | every file |
| `"folders"` | every folder |
| `"background"` | the empty space inside a folder (`%V` is that folder) |
| `".md"` | files with that extension (types that run code, like `.exe`, are refused) |

Windows 11 shows these under "Show more options"; the short menu is reserved
for apps with a packaged shell extension. Selecting several files starts one
copy of your app per file. Setup only changes entries with its own mark, and
leaves an entry of the same name that another program made.

## [[migrate]]

Moving an app from Inno Setup, NSIS or an MSI to Bach would leave it on every
upgraded computer twice: once from the old install, once from the new one.
`[[migrate]]` names the old install, and a **first** install with Bach removes
it before it places any file, using the uninstall command Windows already holds
for it under Settings › Apps.

```toml
[[migrate]]
uninstall_key = "{8A69D345-D564-463C-AFF1-A69D9E530F96}_is1"   # Inno Setup: your AppId, then _is1

[[migrate]]
uninstall_key = "Hello"          # NSIS: the key your script wrote under Uninstall
arguments = "/S"                 # quiet switch, when the entry has no QuietUninstallString

[[migrate]]
uninstall_key = "{3F2504E0-4F89-11D3-9A0C-0305E82C3301}"   # an MSI: its product code
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `uninstall_key` | required | The name of the old install's entry under `Software\Microsoft\Windows\CurrentVersion\Uninstall`, in the user's or the computer's registry (also the 32-bit view). Inno Setup writes `<AppId>_is1`, an MSI its product code, NSIS whatever name your script chose (see `regedit`). |
| `arguments` | none | Arguments that make the old uninstaller run without a window, added after its `UninstallString`: `/VERYSILENT /NORESTART` for Inno Setup, `/S` for NSIS. Without them setup uses the entry's `QuietUninstallString`; with neither it leaves the old install alone. An MSI is always removed quietly (`msiexec /x {code} /qn /norestart`). |
| `elevated` | `false` | Also remove an old install that is machine-wide (listed under `HKLM`): Windows asks for administrator rights. Without it, setup leaves a machine-wide old install alone, because a per-user setup can't remove it. |

What setup does and doesn't do:

- It runs only the command in that entry, never through a shell, and only if
  it's an absolute path to an `.exe` that exists. It waits for the old
  uninstaller (up to 15 minutes) and then until its entry is gone from Settings
  › Apps (up to 90 seconds), because some uninstallers hand over to a copy of
  themselves and exit at once.
- The command comes from the registry, so what may run with administrator
  rights is limited. A per-user old install (listed under `HKCU`, which the
  user's own programs can write) is never run by an elevated setup, such as a
  machine-wide install. A machine-wide old install (under `HKLM`) is run only
  with `elevated = true` (or when setup is elevated already), and only if its
  uninstaller sits in Program Files or the Windows folder. `%SystemRoot%` and
  `msiexec` come from Windows, not from environment variables. Anything else
  is left alone, with a line in the log.
- It never stops an install. An old install that isn't there, can't be removed
  silently or fails to go is written to the log (`%TEMP%\<app id>-setup.log`)
  and left alone, and the new version installs anyway.
- It doesn't touch the user's data: the old uninstaller decides what it
  removes, and most keep settings. Keep `app.id` and `uninstall.data` pointed
  at the folders the old app used so the new version finds them.
- It runs on a first install only, not on updates. If the old install and the
  new one would use the same folder, the old one is removed first, so nothing it
  deletes can be a file Bach just installed.

## [[shortcuts]]

More Start menu shortcuts, next to the app's own: a second program in the
install folder, or the app with arguments. They follow the Start menu box,
and go away when their file isn't installed (an unticked component).

```toml
[[shortcuts]]
name = "Hello Docs"
target = "docs/viewer.exe"       # relative to [files] source
arguments = "--welcome"           # optional
description = "Read the manual"   # optional tooltip
```

## [updates]

Lets the installed app update itself. `bach build` writes `latest.json`
next to the setup file; publish both, and the app finds new versions. With
GitHub releases, one line is enough:

```toml
[updates]
github = "owner/repo"
```

or with a server of your own:

```toml
[updates]
feed = "https://example.com/hello/latest.json"
download_url = "https://example.com/hello/{file}"   # {file}: the setup file's name
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `github` | none | `owner/repo`. Updates come from that repository's releases: the feed is `https://github.com/owner/repo/releases/latest/download/latest.json`, and each version's file is downloaded from its own tag, `.../releases/download/v{version}/{file}`. The setup file is named the way GitHub serves it (anything but letters, digits, `.`, `-` and `_` becomes `-`). |
| `feed` | from `github` | The `https` link of `latest.json`, which the installed app reads. Needed without `github`. |
| `download_url` | from `github` | The `https` link the setup file will be downloaded from. `{file}` is the setup file's name (encoded for a link), `{version}` the app's version. |
| `mirrors` | none | More feeds to ask when `feed` can't be reached, each the `https` link of a full feed with its own download link. The first feed that answers decides the newest version; a mirror only offers another place to download the *same* file (same version and SHA-256), so a weaker mirror can't widen what an installed app trusts. |
| `auto` | `true` | Keep the app up to date in the background: `true` (the install page offers "Automatic updates", ticked), `false` (offered, unticked), `"always"` or `"never"`. |

Either `github` or `feed` is needed, and `app.version` must be numbers like
`1.4.0` (a suffix like `-beta.1` is fine), because the app compares versions.

To roll an update out to a share of computers first, write the feed with
`bach feed SETUP.exe --rollout 10`; see [updates](updates.md#staged-rollouts).

Signing changes the setup file, and so its SHA-256: after signing, write the
feed again with `bach feed SETUP.exe`. The whole story, from publishing with
the GitHub action to what happens on the user's PC, is in
[updates](updates.md); the commands are under
[updating from the app](#updating-from-the-app).

## [access]

Asks for an access key before installing, and encrypts the packaged files
with it, so a setup file can be handed out or hosted publicly without its
contents being readable. The key is **never written in `bach.toml`**: the
build reads it from an environment variable (or `--key-file`).

```toml
[access]
get_key_url = "https://example.com/buy"   # optional: the key page gets a "Get a key" button
```

```bash
bach key                                   # K7QF-3MWD-9XPA-H2TY-6NBC
BACH_ACCESS_KEY=K7QF-3MWD-9XPA-H2TY-6NBC bach build
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `encrypt` | `true` | Encrypt the packaged files with the key. |
| `key_env` | `BACH_ACCESS_KEY` | The environment variable the build reads the key from. `bach build --key-file FILE` reads its first line instead. |
| `get_key_url` | none | An `https` page where people get a key. |
| `send_key` | `true` with `[updates]` | Send the key as `Authorization: Bearer <key>` with every request to the host of `[updates] feed` (the feed, and a download on that same host, redirects from it included), so a private server can refuse anyone without one. Mirrors and downloads on other hosts never get it. |

A key is 8 to 128 characters: letters, digits and `- _ . ~ + / = :`.
`bach key` makes 100-bit random ones (`--count 100` for a batch, say one per
customer). Build one setup file per key to hand each customer their own, or
share one key.

Setup asks for the key on its own page before the license (typed or pasted),
and a wrong key stops before anything on the computer is touched. A silent
install takes it from `--key-file FILE` or the `BACH_ACCESS_KEY` environment
variable (never an argument, which other programs can read): without one it
exits with code 2, with a wrong one with code 1.

The key goes only to the host of the main update feed, which is in the signed
setup file: a feed can name any download link, so a link on another host is
fetched without it. Host your private downloads on the feed's host (or redirect to
a signed storage link from it).

After a successful install the key is kept, sealed with Windows' Data
Protection API for that Windows user (`HKCU\Software\Bach\Access`), so
updates can send it and open protected setup files; removing the app deletes
it. `[access]` with `encrypt = false` only sends the key to your server.

How it is protected: the key goes through Argon2id (64 MiB, 3 passes) into
a ChaCha20-Poly1305 key, and the archive is encrypted in 64 KiB chunks that are
each authenticated, so a changed, reordered or cut-short file fails instead of
unpacking something wrong. What it does **not** hide: file names and sizes
(the manifest lists them), and anything after install, because an installed
app's files are ordinary files on the user's disk. The key protects the
setup file, not a running app: for that, check a license inside your app.
With the [online installer](#online-and-offline-installers) the files never
sit in the setup file at all, and with `send_key` the download itself is
behind your server's check.

## [text]

Replaces any text the window shows. Give one text for every language, or a
text per language:

```toml
[text]
install_title = { en = "Get {name}", de = "Hol dir {name}" }
install_subtitle = "The fastest way to write things down."
footer = "Made with care by {publisher}"
```

Every text can use `{name}`, `{version}` and `{publisher}`. A few have their
own placeholders, listed below. When the window speaks several languages and
a text is given in only one, `bach check` points it out.

| Text | English default |
| --- | --- |
| `window_install` | {name} Setup |
| `window_remove` | Remove {name} |
| `install_title` | Install {name} |
| `install_subtitle` | (the app's description; `{description}` works here) |
| `install_button` | Install |
| `reinstall_title` | Reinstall {name} |
| `reinstall_subtitle` | {name} {version} is already installed. Reinstalling keeps your data. |
| `reinstall_button` | Reinstall |
| `update_title` | Update {name} |
| `update_subtitle` | {name} {old_version} is installed. This updates it to {version} and keeps your data. |
| `update_button` | Update |
| `remove_title` | Remove {name} |
| `remove_subtitle` | This removes {name} and its shortcuts from this computer. |
| `remove_button` | Remove |
| `license_title` | License agreement |
| `license_subtitle` | Please read the terms before you install {name}. |
| `license_accept` | I agree |
| `cancel`, `close`, `retry`, `change` | Cancel, Close, Try again, Change |
| `open`, `open_short` | Open {name}, Open |
| `footer` | {publisher} · Version {version} |
| `preview` | Preview |
| `location` | Location |
| `choose_folder` | Choose where to install {name} |
| `desktop_shortcut`, `desktop_shortcut_description` | Desktop shortcut, Start {name} from your desktop too. |
| `start_menu`, `start_menu_description` | Start menu, Find {name} in the Start menu. |
| `start_with_windows`, `start_with_windows_description` | Start with Windows, Open {name} automatically when you sign in. |
| `auto_update`, `auto_update_description` | Automatic updates, Install new versions of {name} in the background while it isn't running. |
| `add_to_path`, `add_to_path_description` | Add to PATH, Run {name} from any terminal. |
| `delete_data`, `delete_data_description` | Also delete my data, Settings and files {name} saved on this computer. |
| `installing`, `updating`, `removing` | Installing {name}, Updating {name}, Removing {name} |
| `step_preparing`, `step_copying`, `step_tidying`, `step_uninstaller`, `step_shortcuts`, `step_registering` | The progress labels while installing. |
| `step_removing`, `step_removing_shortcuts`, `step_unregistering`, `step_deleting_data`, `step_cleaning`, `step_done` | The progress labels while removing. |
| `installed_title`, `installed_subtitle`, `installed_subtitle_plain`, `updated_title` | {name} is installed, Find it in the Start menu, or open it now., {name} is ready to use., {name} is up to date |
| `removed_title`, `removed_subtitle`, `removed_data_deleted`, `removed_data_kept` | The last page after removing. |
| `install_failed`, `update_failed`, `remove_failed` | {name} couldn't be installed (updated, removed) |
| `error_still_open`, `error_in_use` (`{path}`), `error_busy`, `error_disk_full` (`{size}`, `{free}`), `error_no_access` (`{path}`) | The errors people meet most. |
| `error_create`, `error_write`, `error_delete`, `error_shortcut`, `error_delete_data`, `error_not_found` (all `{path}`) | Errors about one file or folder; Windows' own reason follows them. |
| `error_read_setup`, `error_damaged`, `error_uninstaller`, `error_register`, `error_unregister`, `error_system`, `error_open_failed` (`{error}`) | The rarer errors. |
| `problem_title`, `problem_damaged` (`{error}`) | The page a damaged setup file shows. |
| `downgrade_title`, `downgrade_subtitle` (`{old_version}`), `downgrade_button` | Installing over a newer version. |
| `close_app`, `error_close_app` | "Close and continue", and what it says when the app won't close. |
| `requirement_title`, `requirement_subtitle`, `requirement_button` (all `{requirement}`) | The page for a missing requirement. |
| `step_downloading`, `error_download` (`{error}`) | Updating from the app. |
| `step_finding`, `step_fetching`, `error_fetch` (`{error}`) | The online installer's labels, and what it says when it can't download. |
| `access_title`, `access_subtitle`, `access_hint`, `access_button`, `access_paste`, `access_get`, `access_wrong` | The access key page. |
| `step_running`, `error_run` (`{task}`, `{code}`) | A `[[run]]` step's label and its failure. |
| `step_migrating` | The label while an older install by another installer is removed (`[[migrate]]`). |
| `website`, `feedback` | The link buttons on the last page. |

`crates/pack/src/i18n/en.rs` has every English text in full, and the other
files there every translation.

## [uninstall]

| Setting | Default | Meaning |
| --- | --- | --- |
| `data` | none | Folders the app keeps its data in. The uninstaller offers "Also delete my data" (unticked) and deletes them only when the user ticks it. Each must start with `%APPDATA%`, `%LOCALAPPDATA%` or `%USERPROFILE%`, followed by your app's own folder, so the uninstaller can only ever delete inside the user's profile. |
| `data_description` | a generic line | What lives in those folders, in a few words, shown under the checkbox. Text or `{ en = "…", de = "…" }`. |
| `feedback_url` | none | An `https` page the uninstaller's last page links to ("Tell us why"). |

Write Windows paths in TOML with single quotes so backslashes stay as they
are: `data = ['%APPDATA%\My App']`.

## [signing]

Optional. Says how to sign the setup files, so `bach build --sign` can do it
instead of you running a signing tool. Without this section nothing is signed.

```toml
[signing]
program = "signtool"      # default
arguments = '/fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a'
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `program` | `signtool` | The signing program, started without a shell. Give its full path if it isn't on `PATH`. |
| `arguments` | required | Its arguments, split like a command line. The setup file is added last; write `{file}` to put it elsewhere. `{env:NAME}` is replaced by an environment variable, so secrets stay out of `bach.toml` (the build fails when it isn't set). |

Signing runs only when you ask: `bach build --sign` (or `BACH_SIGN=1` in CI)
signs what it writes, and `bach sign FILE` signs a file you built earlier. A
`bach.toml` never starts a program by itself. Signing happens before the
update feed is written. Details,
and why the installed `uninstall.exe` stays unsigned, are in
[signing](signing.md).

## A complete example

```toml
[app]
name = "Hello"
version = "1.4.0"
publisher = "Hello Inc."
executable = "hello.exe"
description = "Says hello, nicely."
website = "https://hello.example"
icon = "assets/hello.ico"

[files]
source = "target/release/package"
exclude = ["*.pdb"]

[installer]
output = "dist/Hello-Setup-{version}.exe"
theme = "system"
accent = "#2563eb"
logo = "assets/logo.svg"
languages = ["en", "de"]
license = { en = "LICENSE.txt", de = "LIZENZ.txt" }
desktop_shortcut = true
start_menu_shortcut = "always"
start_with_windows = false
add_to_path = false
path_folder = "bin"

[[components]]
id = "samples"
title = { en = "Sample projects", de = "Beispielprojekte" }
files = ["samples/**"]
default = true

[[protocols]]
scheme = "hello"

[[file_types]]
extension = ".hello"
description = "Hello note"

[text]
install_subtitle = { en = "Say hello in style.", de = "Sag Hallo mit Stil." }

[uninstall]
data = ['%APPDATA%\Hello', '%LOCALAPPDATA%\Hello\Cache']
data_description = { en = "Your settings and the download cache.", de = "Deine Einstellungen und der Download-Cache." }
```

## Silent installs

Every setup file also installs without a window, for IT tools and scripts:

```bash
Hello-Setup-1.4.0.exe --silent
```

| Flag | Meaning |
| --- | --- |
| `--silent` (also `/S`, `/quiet`) | No window. Exit code 0 when done, 1 when it failed (the reason is printed in a terminal), 2 for a command-line mistake. |
| `--dir PATH` | Install here instead of the default folder (a first install, when `change_folder` allows it). |
| `--with a,b` / `--without a,b` | Tick or untick options: `desktop-shortcut`, `start-menu`, `autostart`, `path`, `auto-update`, and component ids. Only options the install page would offer. |
| `--lang CODE` | The language for messages, instead of Windows' own. |
| `--allow-downgrade` | Install over a newer version (refused otherwise, with exit code 1). |
| `--key-file FILE` | The access key (first line of the file) for a setup file with [`[access]`](#access). The `BACH_ACCESS_KEY` environment variable does the same. Without a key a silent run exits with code 2. |
| `--log FILE` | Where the log goes. By default every run writes `%TEMP%\<app id>-setup.log`. |
| `--help` (also `-h`, `/?`) | Prints every option. |

A silent run accepts the license on the user's behalf and stops when a
requirement is missing. Settings › Apps runs `uninstall.exe --silent` for a
quiet removal; add `--delete-data` to delete the user's data too.

In the window, Tab and the arrow keys move between the choices and buttons,
Space or Enter does what a click would, and Escape cancels. When the app is
still open, the error page offers "Close and continue", which asks the app
to close the way Windows Update does (it may ask to save first; nothing is
killed).

## Updating from the app

With `[updates]`, the installed app can ask its own uninstaller, which
knows the feed:

```bash
"%LOCALAPPDATA%\Programs\Hello\uninstall.exe" --check-update
```

| Flag | Meaning |
| --- | --- |
| `--check-update` | Exit code 10 when a newer version is out (it prints which), 0 when up to date (or when a [staged rollout](updates.md#staged-rollouts) hasn't reached this computer yet), 1 when the feed can't be read. Changes nothing. |
| `--check-update --json` | Prints one line of JSON instead: `app`, `current`, `latest`, `available`, `downloaded` (kept ready already), `notes`, `size`. |
| `--download-update` | Downloads a newer version into `%LOCALAPPDATA%\Bach\updates\<app id>` and keeps it ready. Exit code 10: ready, 0: up to date. A cut-off download continues where it stopped. |
| `--apply-update` | Installs what was kept ready, silently. Exit code 0: installed, or nothing was waiting; 1: it couldn't be installed yet (it stays ready). |
| `--wait-for PID` | With `--apply-update` or `--update`: wait up to ten minutes for that process (your app) to end first. |
| `--launch` | With `--apply-update` or `--update --silent`: start the app again afterwards. |
| `--update` | Downloads it (or uses the one kept ready) and opens its setup window. |
| `--update --silent` | Installs it without a window and exits with the setup's own code. |
| `--auto-update` | One background round: check, download, and install if the app isn't running. This is what the "Automatic updates" scheduled task runs. |

Updating in the background, so closing the app is all the user does: run
`--download-update` when the app starts, and `--apply-update --wait-for
<your process id> --launch` when it quits.

Downloads are https only, checked against the feed's SHA-256, and must be a
setup file for the same app id and the version the feed names, so a wrong or
tampered file is never run. A kept download is checked again right before it
runs. An update cut short by a power cut is put back by the next install,
update or removal. Every run logs to `%TEMP%\<app id>-update.log`.

## Editor support

`bach init` starts the file with

```toml
#:schema https://raw.githubusercontent.com/usebach/bach/main/schema/bach.schema.json
```

so editors with TOML support (VS Code with Even Better TOML, Zed, JetBrains
with Taplo) complete every setting and flag typos as you type.

## Listing your app in winget

```bash
bach winget dist/Hello-Setup-1.4.0.exe --url https://example.com/Hello-Setup-1.4.0.exe --license MIT
```

writes the three manifests the Windows Package Manager needs; check them
with `winget validate` and open a pull request to microsoft/winget-pkgs.
