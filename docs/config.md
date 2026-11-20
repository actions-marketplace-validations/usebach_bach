# bach.toml reference

Every setting Bach reads. Paths are relative to the `bach.toml` file. Unknown
settings are an error, so a typo never goes unnoticed, and every mistake is
reported as one sentence naming the setting at fault.

Only `[app]` and `[files]` are required. Everything else has a sensible
default, so a five-line config already builds a complete installer.

- [`[app]`](#app): name, version, publisher, the program
- [`[files]`](#files): what gets installed
- [`[installer]`](#installer): look, languages, license and options
- [`[style]`](#style): the window's style, and your own fonts, colours and background
- [`[layout]`](#layout): ten layout presets, and the window's size, panel, spacing and animation
- [`[[components]]`](#components): optional parts the user can leave out
- [`[[protocols]]`](#protocols): links like `myapp://` that open your app
- [`[[file_types]]`](#file_types): files your app opens
- [`[[requirements]]`](#requirements): what the app needs on the computer first
- [`[[shortcuts]]`](#shortcuts): more Start menu shortcuts
- [`[[run]]`](#run): programs setup starts after the files are in place
- [`[[registry]]`](#registry), [`[[env]]`](#env) and [`[[context_menu]]`](#context_menu): registry values, environment variables and right-click entries
- [`[[migrate]]`](#migrate): remove an older install made by another installer
- [`[updates]`](#updates): updates the installed app fetches itself
- [`[text]`](#text): any text in the window, in any language
- [`[uninstall]`](#uninstall): the user's data
- [`[profiles]`](#profiles): other builds of the app: a company version, a portable ZIP
- [`[arch.arm64]`](#archarm64): one download for x64 and ARM64
- [`[signing]`](#signing): sign the setup files while building (optional)
- [A complete example](#a-complete-example)
- [Silent installs](#silent-installs) and [updating from the app](#updating-from-the-app)
- [Editor support](#editor-support), [listing your app in winget](#listing-your-app-in-winget) and [Chocolatey](#listing-your-app-in-chocolatey)
- [Deploying with Intune](#deploying-with-intune)
- [Checking a setup file](#checking-a-setup-file), [a download page](#a-download-page) and [failing on warnings](#failing-on-warnings)

## config_version

Optional, first in the file: `config_version = 1`. It says which version of this
format the file is written for. Left out, it means the current one, so existing
files keep working. `bach init` writes it. A version this Bach doesn't read stops
`bach check` and `bach build` with one sentence (update Bach for a newer one).

## [app]

| Setting | Required | Meaning |
| --- | --- | --- |
| `name` | yes | The app's name, as people see it: the window, the Start menu, the desktop shortcut and Settings › Apps. |
| `version` | yes | Any version text, like `1.2.0` or `2.0.0-beta.1`. Shown in the window and Settings › Apps; the first numbers also go into the setup file's details. |
| `publisher` | yes | You or your company. Shown in the window's footer and Settings › Apps. |
| `executable` | yes | The program to start, relative to `[files] source`, for example `my-app.exe` or `bin/my-app.exe`. Shortcuts and the "Open" button start it. |
| `id` | no | Identifies the install across versions (registry key name). Defaults to `name`. Letters, digits, spaces, `.`, `-` and `_`, up to 64 characters, and not a name Windows keeps for devices (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, with or without an extension, in any letter case). Never change it after your first release, or updates install side by side. |
| `description` | no | One line. The shortcut's tooltip, and the line under "Install My App" on the first page (unless `[text] install_subtitle` replaces it). Without it, that line is left out. |
| `website` | no | An `https://` link, shown as the support link in Settings › Apps. |
| `icon` | no | A `.ico` file. It becomes the setup file's icon and the window's icon, and, when it holds a PNG image (most modern icons do) and `installer.logo` isn't set, the logo in the window. Include sizes 16 to 256 for crisp results. |

## [files]

| Setting | Required | Meaning |
| --- | --- | --- |
| `source` | yes | The folder whose contents are installed, subfolders included. |
| `exclude` | no | Patterns to leave out. `*` matches within a name, `**` across folders, `?` one character. A pattern without `/` matches file names anywhere (`*.pdb`); one with `/` matches from the top of `source` (`docs/**`). Case doesn't matter. |
| `allow_secrets` | no | Patterns (same rules as `exclude`) for files that look like secrets but are meant to ship. |

`bach check` and `bach build` stop when the source folder holds a file that
looks like a secret: `.env`, `.env.*` and `*.env`, `*.pfx`, `*.p12`, `*.pem`,
`*.key`, `*.ppk`, `*.p8`, `*.snk`, `*.jks`, `*.keystore`, `*.kdbx`, `id_rsa*`,
`id_dsa*`, `id_ecdsa*`, `id_ed25519*`, `secring.gpg`, `trustdb.gpg`,
`*.tfstate`, `*.tfvars`, `credentials` and `credentials.json`,
`service-account*.json`, `local.settings.json`, `.git-credentials`, `.npmrc`,
`.yarnrc.yml`, `.pypirc`, `.netrc`, `.htpasswd`, `.dockercfg`, `.pgpass`,
`.vault-token`, `.aws-credentials`, `.bash_history`, `.zsh_history`,
`wallet.dat`, `secrets.json`, a file called `.git`, a `bach.toml` (or
`bach-*.toml`), and anything inside a `.git`, `.svn`, `.hg`, `.ssh`, `.aws`,
`.azure`, `.gnupg`, `.docker`, `.kube` or `.terraform` folder, plus
`.m2/settings.xml`, `.gradle/gradle.properties`, `.gem/credentials` and
`.config/gcloud`. Small text files in configuration formats (`.json`, `.yml`,
`.yaml`, `.toml`, `.ini`, `.cfg`, `.conf`, `.config`, `.properties`, `.txt`,
`.xml`, `.env`, `.pem` and scripts, up to 256 KiB) are also read for a private
key block, an AWS access key (`AKIA...`), a GitHub token (`ghp_...`,
`github_pat_...`) or a Slack token (`xoxb-...`); programs and libraries are
never read. The message names the first file and why. Leave the file out with
`exclude`, or name it in `allow_secrets` to ship it on purpose. Files that only
add weight (`*.pdb`, `*.log`, `Thumbs.db`, `.DS_Store`, and anything in
`node_modules`, `target` or `__pycache__`) give a warning, not an error. `bach
init` excludes the first four for the stacks it recognises.

**Long paths.** Windows' classic limit is 260 characters for a whole path.
Setup writes longer paths fine, but shortcuts, Explorer and many programs stop
there unless Windows' long path support is on. `bach check` adds up the usual
install folder (`%LOCALAPPDATA%\Programs\<folder>` for one user, with a
20-character user name, or `C:\Program Files\<folder>`) and the longest
packaged path, and warns when the total passes 259 characters; the setup log
says so too when it happens on a computer. Flatten deep trees such as
`node_modules`, or build the app so its files sit shallower.

Every name in the source folder, file or folder, has to be one Windows can
create: at most 255 characters (counted as Windows counts them) and not a
device name such as `NUL` or `COM1`. `bach check` reports the first one that
isn't, before a setup file is built.

The source folder may not contain symbolic links or a file named
`uninstall.exe` at its top (Bach adds its own).

## [installer]

### Look

| Setting | Default | Meaning |
| --- | --- | --- |
| `output` | `dist/{name}-Setup-{version}.exe` | Where the setup file goes. `{name}`, `{version}` and `{id}` are filled in. |
| `theme` | `system` | `system` follows Windows' light or dark setting, live. `light` or `dark` fixes it. A style that looks best dark (see [`[style]`](#style)) starts dark unless you set this. |
| `style` | `bach` | The window's built-in style, like `"aurora"` or `"arcade"`: its colours, fonts and corners. See [`[style]`](#style) for all 14 and how to change them. |
| `accent` | none (black / white) | A colour like `#2563eb` for buttons, the progress bar and ticked boxes. The button text picks black or white for contrast. |
| `logo` | your icon, else the Bach mark | An `.svg` or `.png` shown at the top of the window, or `"none"` for no logo at all. An SVG is drawn in one colour that follows the theme; use a PNG for a full-colour logo. A wide PNG (a wordmark) keeps its shape. Give a PNG twice the size it's shown at for sharp results. |
| `logo_height` | `56` | The logo's height in points, 32 to 96. |
| `online` | `false` | `true` also builds the **online installer** (see below). Needs [`[updates]`](#updates). `bach build --online` does the same once. |
| `online_output` | `dist/{name}-Web-Setup.exe` | Where the online installer goes. No version in the name, so one download link keeps pointing at the newest. |
| `compression` | `best` | `best` makes the smallest file, `normal` and `fast` build quicker. Unpacking is fast either way. `bach build --fast` overrides it once. |
| `compression_threads` | as many as the memory budget allows | How many threads compress, from 1 to 256 (never more than the processor cores). The setup file is the same whatever the number. |
| `compression_memory` | `1024` | The memory budget for compressing, in MiB (64 to 65536). It holds the number of threads back, but one thread always runs. When it does, the build says so. |

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
Wrote dist\Hello-Setup-1.4.0.exe (8.7 MB)
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
| `folder` | the app id | The install folder's name under `%LOCALAPPDATA%\Programs`. When the user picks another place, this name is added to it. Same rules as `app.id`. |
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
- The install folder has to be inside Program Files (the default). A machine-wide
  app runs, updates and uninstalls with administrator rights, so setup refuses a
  folder the user's own programs could write to, with the usual "choose another
  folder" message. Even inside Program Files, setup reads the folder's access
  list before it writes, and again before an elevated update or removal: a
  folder that belongs to someone other than Administrators, SYSTEM or
  TrustedInstaller, or that Users, Authenticated Users or Everyone may change,
  is refused with `error_unsafe_folder`, and the log names the reason.
- A protected setup file (`[access]`) run silently should get its key with
  `--key-file`: the elevated copy is sure to get the arguments, and the
  `BACH_ACCESS_KEY` variable may not reach it.
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

Every shortcut setup writes (the Start menu one, the desktop one and those of
`[[shortcuts]]`) carries the app's AppUserModelID: the app id with every
character other than letters, digits, `.`, `_` and `-` turned into a `.` (at
most 128 characters), so `My App` becomes `My.App`. Windows uses it to group
the app's windows on the taskbar and to tie its notifications to the app. It
does not change with the version, and it lives in the shortcut file, so
removing the shortcuts leaves nothing behind. For the grouping to follow a
window, your program has to use the same ID
(`SetCurrentProcessExplicitAppUserModelID`) or leave it to Windows.

The whole row is a click target: clicking anywhere on it ticks or unticks
its box. On an update, the boxes start the way the user left them last time,
and unticking one takes the shortcut, PATH entry or autostart away again.

### Languages and license

| Setting | Default | Meaning |
| --- | --- | --- |
| `languages` | every built-in language | The languages the window may speak, like `["en", "de"]`. The window follows the user's Windows display language when it's in this list, and otherwise uses the first one. With a single language, like `["de"]`, the window always speaks it. |
| `license` | none | A UTF-8 text file the user accepts before the install page, like `"LICENSE.txt"`, or one per language: `{ en = "LICENSE.txt", de = "LIZENZ.txt" }`. An update shows it again only when it changed. The window shows it in a card that scrolls smoothly, with a scrollbar, and the text can be selected and copied. |

Built-in languages: `en` English, `de` Deutsch, `nl` Nederlands, `fr`
Français, `es` Español, `it` Italiano, `pt` Português (Brazilian), `pl` Polski, `ru`
Русский, `tr` Türkçe, `ja` 日本語, `ko` 한국어, `zh` 简体中文, `zh-tw` 繁體中文 (Traditional Chinese, for Taiwan, Hong Kong and Macau; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant` tags pick it, and `zh-CN`, `zh-SG` and `zh-Hans` pick `zh`), `sv` Svenska, `cs` Čeština, `vi` Tiếng Việt, `id`
Bahasa Indonesia, `hi` हिन्दी.

## [style]

The window comes in 14 built-in styles. Pick one with `installer.style`, then
change whatever you like in `[style]`: the font, every colour, the corners,
a gradient or a picture behind the window.

```toml
[installer]
style = "arcade"
```

`bach styles` lists them, and `bach preview --style NAME` shows one on your
own app without changing `bach.toml`.

| Style | What it looks like | Suits |
| --- | --- | --- |
| `bach` | Bach's own look: white or graphite, black buttons, Inter. The default. | any app |
| `aurora` | A soft violet-to-blue wash, frosted cards and pill buttons. | Tauri, Electron, Wails and Flutter apps |
| `terminal` | A monospaced face (Cascadia Mono), near-black with a green glow on every edge, and a green prompt. Dark by default. | command-line tools |
| `arcade` | Neon pink on a deep violet glow, with a heavy heading and a dark-on-pink button. Dark by default. | indie and web-tech games |
| `forge` | Brushed steel and molten orange, with tight corners. Dark by default. | 3D games made with Unity or Unreal |
| `lagoon` | Calm sea blue on a pale wash, rounded and friendly. | games and tools made with Godot |
| `pixel` | Square corners, a monospaced face, a night-blue screen and a warm gold frame. Dark by default. | pixel-art and retro games |
| `midnight` | Deep navy fading to black, blue-tinted glass cards and an electric blue button. Dark by default. | developer tools |
| `paper` | Warm paper, a serif heading (Georgia) and a terracotta accent. | writing, reading and note apps |
| `ember` | Charcoal with a warm amber glow. Dark by default. | audio, video and creative tools |
| `forest` | Moss and sage on a soft green wash, cream cards and pill buttons. | productivity, health and outdoor apps |
| `blossom` | Soft pink, generous corners and pill buttons. | friendly consumer apps |
| `mono` | Black and white only: hairlines, small corners and a heavy heading. | strict, plain utilities |
| `frost` | Cool blue-grey, calm and quiet. | system utilities, backup and sync tools |

Every style has a light and a dark version and follows `installer.theme`;
the ones marked "dark by default" start dark when `theme` isn't set. Each
uses fonts every Windows 10 and 11 PC has (Cascadia Mono or Consolas,
Bahnschrift, Georgia) and falls back to Inter, which every setup
file carries, when one is missing.

The default is always `bach`, whatever kind of app you ship: `bach init`
doesn't pick a style for you. The "Suits" column is only a hint; any
style works for any app, and you switch by setting `installer.style`.

### Your own look

Everything in `[style]` is optional and goes on top of the chosen style.
Colours set in `[style]` apply in light and dark mode; `[style.light]` and
`[style.dark]` set them for one mode and win.

```toml
[installer]
style = "aurora"

[style]
font = "fonts/Outfit.ttf"       # or a font Windows has, like "Segoe UI"
heading_weight = 700
radius = 20
button_radius = 16              # pill buttons
accent = "#22d3ee"
background_image = "art/backdrop.png"
background_image_opacity = 0.4

[style.dark]
background = ["#1d0b36", "#0a0416"]   # a gradient from top to bottom
card = "#ffffff0b"                    # the window shows through

[style.light]
background = "#ffffff"
text = "#1b1b1b"
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `font` | the style's | The text's font: a family Windows has, like `"Segoe UI"`, or a `.ttf`/`.otf` file in your project, which ships in the setup file. A list of files (`["fonts/Nunito-Regular.ttf", "fonts/Nunito-Bold.ttf"]`) ships several cuts of one family. A family the PC doesn't have falls back to the style's font, then Inter. Web fonts (`.woff`, `.woff2`) and collections (`.ttc`) aren't supported. Up to 4 MB per file. |
| `heading_font` | `font`, else the style's | The headings' font, in the same forms as `font`. |
| `heading_weight` | the style's, or the heaviest font file's | 100 to 900: 400 regular, 600 semibold, 700 bold. |
| `radius` | the style's | Corners of the card, the license box and the key field, 0 (square) to 24 points. |
| `button_radius` | the style's | Corners of buttons, icon wells and checkboxes, 0 to 16 points. 16 makes pill buttons. |
| `background_image` | none | A PNG or JPEG drawn over the background colour, covering the window (up to 8 MB). Pick a calm picture: the text sits on it. |
| `background_image_opacity` | `0.35` | How strongly the picture shows, 0 to 1. |
| `background` | the style's | The window's colour, or two colours for a gradient from top to bottom. Opaque only. |
| `card` | the style's | The card of choices, the license box and the key field. `#rrggbbaa` lets the background show through. |
| `text` | the style's | Headings and labels. |
| `muted_text` | the style's | Descriptions, the subtitle and the footer. |
| `accent` | the style's | Buttons, the progress bar and ticked boxes. Same as `installer.accent`; set one of them, not both. |
| `accent_text` | black or white | Text on the accent; by default whichever reads. |
| `border` | the style's | The card's edges and its dividers. |
| `error` | the style's | The remove button and failure icons. |
| `mark` | the text colour or the accent | The Bach mark and one-colour (SVG) logos. Styles that colour the mark keep it in the accent, also when you change the accent. |
| `panel` | a light tint of the accent | The panel of [`[layout]`](#layout) where it shows no picture. |
| `panel_text` | whatever reads on the panel | The heading and subtitle on the panel; light on a `panel_image`. |

Colours are `#rrggbb`, or `#rrggbbaa` to let what's behind show through.
The quieter colours (icon wells, hover washes, the progress track, checkbox
edges) are mixed from these, so the window stays consistent whatever you set.
Fonts and the picture count toward the 16 MB the logo, fonts and picture may
take together. `bach check` shows the look in one line:

```console
$ bach check
  ...
  Look         arcade style, dark, font Outfit, own colours, own corners, 1 font file, background picture
```

## [layout]

How the window is sized, arranged and animated, on top of its style.
Everything is optional; without `[layout]` the window is the classic
column it always was. The quickest start is a preset, and every setting
below changes it:

```toml
[layout]
preset = "showcase"               # the choices on the left, a rounded picture on the right
panel_image = "art/hero.jpg"
heading_size = 34                 # anything else you want different
```

`bach layouts` lists the presets, and `bach preview --layout NAME` shows
one on your app without changing `bach.toml` (`bach watch` reopens the
window on every save). Texts are changed in [`[text]`](#text), and which
options the window offers in [`[installer]`](#options).

### Presets

| Preset | What it looks like | Best with |
| --- | --- | --- |
| `classic` | One calm centred column. The default. | |
| `showcase` | The choices on the left, a large picture on the right, inset with rounded corners. | `panel_image` |
| `spotlight` | Key art down the left with the heading on it, the choices as tiles beside it. | `panel_image` |
| `hero` | A wide rounded picture on top with the heading on it, the choices as tiles below. | `panel_image` |
| `banner` | A picture across the top, edge to edge, with the heading on it; the choices below. | `panel_image` |
| `floating` | Everything on one card floating over the window's picture or colour. | `[style] background_image` |
| `editorial` | A picture edge to edge on the right, a large heading and plain rows on the left. | `panel_image` |
| `portrait` | A tall, narrow window: a large rounded picture with the heading on it, the choices below. | `panel_image` |
| `tiles` | A wide window with the install folder across the top and the options as tiles below. | |
| `minimal` | No card and no icons: the rows sit on the window, divided by hairlines. | |

A preset that is best with a picture still works without one: a panel
without a picture shows the app's logo large on a tint of the accent.

### Window and panel

| Setting | Default | Meaning |
| --- | --- | --- |
| `preset` | none | A built-in layout to start from (above). |
| `width` | `540`; `880` with a side panel; `600` with a banner | The window's width in points, 380 to 1400. |
| `height` | as tall as the choices need | The window's height in points, 340 to 1000. Taller than the width makes a portrait window; when the choices don't fit, the card scrolls. The window never grows past 85% of the screen. |
| `panel` | `"none"` | A panel at the `"left"` or `"right"` of the choices, or a banner across the `"top"`. |
| `panel_size` | 46% of the window (40% with the heading on it); `190` for a banner | The panel's width, or a banner's height, in points (120 to 800). The choices keep at least 300 points. |
| `panel_image` | none | A PNG or JPEG that fills the panel, cut to its shape around the middle (up to 8 MB). |
| `panel_inset` | `0` | Space around the panel in points, 0 to 48. With a radius the panel floats as a card. |
| `panel_radius` | `0` | The panel's corners in points, 0 to 48. |
| `header` | `"content"` | With a panel: the logo, heading and subtitle above the choices (`"content"`), or on the `"panel"`. On a picture they sit low over a shade and turn light, unless `[style] panel_text` says otherwise. |
| `content_width` | the room there is | The widest the choices and buttons get, in points (260 to 1200), centred in their room. |
| `surface` | `"plain"` | `"card"` puts the heading, choices and buttons on one card floating over the window's colour or picture. |

The panel's colour without a picture is `[style] panel` (by default a
light tint of the accent), its text `[style] panel_text`.

### Text and spacing

| Setting | Default | Meaning |
| --- | --- | --- |
| `align` | `"center"` (`"left"` with a panel) | How the logo, heading, subtitle and the footer line above the buttons align: `"left"`, `"center"` or `"right"`. |
| `logo` | `"above"` | The logo `"above"` the heading, or `"beside"` it on one line. |
| `padding` | `[44, 32, 24, 32]` | Space inside the window in points, 0 to 96: one number for every side, `[top and bottom, sides]`, or `[top, right, bottom, left]`. |
| `gap` | `28` (`0` with the heading on a side panel) | Points between the heading and the choices. |
| `heading_size` | `22` | The heading's size in points, 14 to 48. A long heading still steps down so it wraps calmly. |
| `text_size` | `14` | The text's size in points, 11 to 20. Descriptions are a point smaller, the footer line two. |

### Choices and buttons

| Setting | Default | Meaning |
| --- | --- | --- |
| `rows` | `"joined"` | `"joined"`: one card with the rows divided by hairlines. `"separate"`: each row a card of its own. `"plain"`: no card, the rows straight on the window with hairlines between them. |
| `columns` | `1` | `2` lays separate rows out as tiles in two columns. |
| `row_padding` | `16` | Points inside each row, 6 to 32. |
| `icons` | `true` | Whether rows show their icon. |
| `dividers` | `true` | Whether joined and plain rows have hairlines between them. |
| `button_height` | `32` | Points, 24 to 56. |
| `buttons` | `"right"` | `"right"`, `"left"`, `"center"`, or `"stretch"` to share the whole width. |
| `primary` | `"last"` | The main button (Install, Remove, Open) `"last"` or `"first"`. Tab follows the order they are drawn in. |
| `footer` | `"beside"` | The footer line (publisher and version, or `[text] footer`): `"beside"` the buttons, `"above"` them, or `"hidden"`. With centred or stretched buttons it goes above them. |

### Motion

| Setting | Default | Meaning |
| --- | --- | --- |
| `animation` | `"rise"` | How each page comes in: `"rise"` (fades in, rising a few points), `"fade"`, `"slide"` (in from the right), `"drop"` (from above) or `"none"`. |
| `animation_speed` | `1` | 0.25 to 4: 2 plays twice as fast, 0.5 half as fast. |

### Your own layout

Every setting works without a preset too. A banner with the heading on
it, tiles in two columns and stretched buttons:

```toml
[layout]
panel = "top"
panel_size = 220
panel_image = "art/banner.jpg"
header = "panel"
align = "left"
width = 760
rows = "separate"
columns = 2
buttons = "stretch"
footer = "above"
animation = "drop"
```

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

### Components that are downloaded

Language packs, HD textures or demo files that most people won't want don't
have to make everybody's download bigger. Add `download` and a component's
files stay out of the setup file; they are downloaded when somebody ticks the
box.

```toml
[[components]]
id = "hd-textures"
title = "HD textures"
files = ["textures/hd/**"]
default = false
download = true                     # or an https link, see below
```

`bach build` packs those files into a file of their own next to the setup file:

```console
Wrote dist/Hello-Setup-1.4.0.exe (11.2 MB)
Wrote dist/Hello-hd-textures-1.4.0.bachpack (412.6 MB), the download for the component hd-textures
```

Upload that file to the link the component names. When the user ticks the box
the setup window says how big the download is ("412.6 MB download"), and
setup downloads the file when it installs, checks it against the SHA-256
recorded in the setup file, and puts the files in place together with the
others: if the download fails, nothing was changed and the page offers to try
again. Cancel works while it downloads.

| `download` | Meaning |
| --- | --- |
| `true` | The link of `[updates]` (`github = "owner/repo"` or `download_url`), with the file's name in place of `{file}`. |
| `"https://…"` | Any https link. `{file}` is the file's name (`Hello-hd-textures-1.4.0.bachpack`), `{version}` the app's version, `{id}` the app id and `{component}` the component's id. |

- **The file is checked, not the host.** Its SHA-256 and size are in the setup
  file, so the link can be any host or CDN: a changed file is refused. Sign
  the setup file and the whole chain is covered.
- **Updates** download again only when a file of the component changed: files
  that are already installed exactly as the new version records them are left
  alone, and an update that leaves the component unticked never downloads it.
- **An online installer** and an update patch carry the same component
  list, so they download the file the same way.
- **`--with hd-textures`** in a silent install downloads it too, and a silent
  install without it never does.
- A setup file that is locked with [`[access]`](#access) can't have a
  downloaded component: the downloaded file would not be locked, so the build
  refuses it. A downloaded component can't contain `app.executable`.
- With a GitHub release, upload the `.bachpack` file as an asset of the same
  release. `bach build --json` lists them as `packs` (`component`, `file`,
  `sha256`, `size`).

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

Removing the app, or an update whose new version drops the type, takes back
what setup wrote: the default value, the entry under `OpenWithProgids` and the
program's own key. The `OpenWithProgids` key and the extension's key go too,
but only when nothing else is left in them, so a program that registered itself
for the same extension keeps its entry.

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

Bach supports Windows 10 version 1809 (build 17763) and newer, and Windows 11.
They are tested on Windows 11, Windows on ARM and Windows Server 2022; Windows
10 itself is not in the automated tests (GitHub has no Windows 10 image). A
`windows` requirement with a `build` below 17763 makes `bach check` and
`bach build` warn, because the setup window may not run there.

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
| `timeout_minutes` | `15` | Minutes before setup ends the step and the programs it started. 1 to 120. |
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
for well-known installers (and sign them), not for anything else. `bach check`
warns about that combination. Right before Windows is asked for administrator
rights, setup checks the file against the size and SHA-256 recorded when the
setup file was built, and doesn't start a program that was swapped since. A step that
runs longer than `timeout_minutes` (15 by default, 1 to 120) is ended together with
everything it started, so no installer it launched keeps files open. The one
exception is an `elevated` step in a setup that isn't elevated itself (a per-user
install): Windows doesn't let a program without administrator rights end one that
has them, so setup only stops waiting, says so in the log, and the program may
keep running until it ends or somebody closes it in Task Manager. What the
program prints is kept in the log, its last 8 KB, so a failure like exit code
1603 comes with its reason. Each run is in the log. `bach inspect`
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

[[registry]]
key = 'Software\Hello\Legacy'
name = "InstallPath"
view = "32"                     # "64" (default), "32" or "both"
value = '{app}'
```

`view` picks the registry view on a 64-bit Windows. A 32-bit program that
reads `HKLM\Software\Vendor` is sent to `HKLM\Software\WOW6432Node\Vendor`,
so a machine-wide app (`installer.scope = "machine"`) whose program is 32-bit
needs `view = "32"` (or `"both"` when it ships 32-bit and 64-bit programs).
Writing, updating and removing use the same view, and `"both"` handles both.
`HKCU\Software` is not redirected, so the value rarely matters there. `bach
check` warns when the install is machine-wide, `app.executable` is a 32-bit
program (its PE header says x86) and a `[[registry]]` value leaves `view` out.

Use single quotes for the key, so the backslashes stay as they are. An
update writes the values again and removes the ones the new version no longer
lists. Removing the app deletes a value only while it still holds what setup
wrote, so a setting the user or the app changed since stays, and keys that end
up empty go too. Keys that belong to Windows, other programs or Bach are
refused: `Software\Classes`, `Software\Policies`, `Software\Bach`, and every
key of the vendors `Microsoft`, `Google`, `Mozilla`, `Mozillaplugins`,
`Chromium`, `RegisteredApplications`, `Clients` and `ODBC` (they hold Office
trusted locations, Defender exclusions, certificates, browser extension hooks
and the like). Use `[[protocols]]`, `[[file_types]]` and `[[context_menu]]`
for what those are for. `bach check` warns when the key isn't below a folder
named after `app.publisher`, `app.name` or `app.id`, which usually means a
typo.

## [[env]]

Environment variables of the user (`HKCU\Environment`). Programs started after
the install see them; ones already running don't.

```toml
[[env]]
name = "HELLO_HOME"
value = '{app}'                 # {app} is the install folder
# allow = true                  # only for a name that doesn't start with HELLO_
```

A name must start with your app's id in capitals, with every character that
isn't a letter or digit turned into `_` (`Hello` gives `HELLO_`, `My App`
gives `MY_APP_`), so the variable can't change what other programs read. Set
`allow = true` to use another name; `bach check` then warns about it.

A value with a `%VARIABLE%` in it is stored so Windows fills it in when it's
read. Removing the app removes a variable only while it still holds the value
setup set. Names Windows uses (`PATH`, `TEMP`, `USERPROFILE`, …) are refused;
`PATH` has its own setting, `add_to_path`. Variables that load code into other
programs or redirect their traffic are refused whatever `allow` says:
`COR_PROFILER` and the other `COR_*` and `CORECLR_*` profiler settings,
`DOTNET_STARTUP_HOOKS`, `JAVA_TOOL_OPTIONS`, `_JAVA_OPTIONS`, `NODE_OPTIONS`,
`PYTHONSTARTUP`, `PYTHONPATH`, `PERL5OPT`, `HTTP_PROXY`, `HTTPS_PROXY`,
`NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `SSLKEYLOGFILE`, `LD_PRELOAD` and a
few more.

## [[context_menu]]

The `name` can be in any script: Chinese, Japanese, Korean, Cyrillic or Arabic
texts work. Setup names the registry key after the text (`Bach.<id>.<text>`);
for a text with no Latin letters it adds the first 8 hex digits of the text's
SHA-256, so two entries that differ only in such characters get two keys. Texts
in Latin script keep the keys they always had, so an update or removal still
finds an earlier install's entries.

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
| `public_key` | none | The publisher's Ed25519 public key (64 hex digits, from `bach feed-key`). With it, the installed app accepts only feed entries signed with the matching private key. See [Signed feeds](updates.md#signed-feeds). |
| `next_public_key` | none | A second public key that is trusted too, to hand over to a new key without stranding installed copies (needs `public_key`, and must differ from it). See [Changing the key](updates.md#changing-the-key). |
| `max_age_days` | none | How many days old a signed feed entry may be (1 to 3650; needs `public_key`). `bach feed` signs the time it ran with each entry, and an installed app ignores an entry older than this, so a taken-over feed host can't keep serving an old, validly signed entry (a version with a known bug, or no newer version at all). Left out: no limit. You have to run `bach feed` again within that time, release or not (a scheduled workflow does it). See [Signed feeds](updates.md#signed-feeds). |
| `beta_feed` | none | The `https` link of the beta channel's feed, written like `feed` (`bach feed FILE --out latest-beta.json`). A person who runs `uninstall.exe --channel beta` follows it instead of `feed` and `mirrors`. See [Beta versions](updates.md#beta-versions). |
| `signer` | none | The publisher a downloaded update has to be signed by (Authenticode): the common name or organisation of the signing certificate, like `Example Inc.` (or its whole subject, `CN=..., O=..., L=..., C=...`, which has to match exactly). Windows must accept the signature, and an unsigned update or one by another publisher is refused. See [Signed updates](updates.md#signed-updates-authenticode). |
| `mirrors` | none | More feeds to ask when `feed` can't be reached, each the `https` link of a full feed with its own download link. The first feed that answers decides the newest version; a mirror only offers another place to download the *same* file (same version and SHA-256), so a weaker mirror can't widen what an installed app trusts. |
| `auto` | `true` | Keep the app up to date in the background: `true` (the install page offers "Automatic updates", ticked), `false` (offered, unticked), `"always"` or `"never"`. |

Either `github` or `feed` is needed, and `app.version` must be numbers like
`1.4.0` (a suffix like `-beta.1` is fine), because the app compares versions.

To roll an update out to a share of computers first, write the feed with
`bach feed SETUP.exe --rollout 10`; see [updates](updates.md#staged-rollouts).
To spare computers the whole download, `bach build --patch-from OLD-SETUP.exe`
also writes a patch with only the files that changed; see
[update patches](updates.md#update-patches).

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

```powershell
bach key --out key.txt                     # writes one key (try --count 100 for a batch)
bach build --key-file key.txt              # every line of the file is a key that opens the setup file
```

`--key-file` works in every shell. The build can also read the key from the
environment variable, which each shell sets its own way:

```powershell
$env:BACH_ACCESS_KEY = "K7QF-3MWD-9XPA-H2TY-6NBC"; bach build   # PowerShell
```

```bat
set BACH_ACCESS_KEY=K7QF-3MWD-9XPA-H2TY-6NBC && bach build      # cmd
```

```bash
BACH_ACCESS_KEY=K7QF-3MWD-9XPA-H2TY-6NBC bach build             # bash, zsh
```

`bach key > key.txt` also works. Windows PowerShell writes such a file as
UTF-16 and Bach reads that, a byte order mark and plain UTF-8 alike.

| Setting | Default | Meaning |
| --- | --- | --- |
| `encrypt` | `true` | Encrypt the packaged files with the key. |
| `key_env` | `BACH_ACCESS_KEY` | The environment variable the build reads the key from (one key). `bach build --key-file FILE` reads every line of the file instead: each is a key, blank lines and lines starting with `#` are skipped, at most 1000. |
| `get_key_url` | none | An `https` page where people get a key. |
| `send_key` | `true` with `[updates]` | Send the key as `Authorization: Bearer <key>` with every request to the host of `[updates] feed` (the feed, and a download on that same host, redirects from it included), so a private server can refuse anyone without one. Mirrors and downloads on other hosts never get it. |

A key is 8 to 128 characters: letters, digits and `- _ . ~ + / = :`.
`bach key` makes 100-bit random ones (`--count 100` for a batch, say one per
customer). A key file with many keys makes one setup file that every one of
them opens on its own: the files are locked with a random key of their own,
and the manifest holds that key once per access key, each locked with it (a
"slot"; the file is stored once, not once per key). To take a customer's key
away, build again without their line and distribute that file: copies already
handed out keep working with the keys they were built with, so pair this with
`send_key` and a server that refuses the revoked key for updates. Opening takes
a third of a second for each slot tried. With 17 keys or more, each slot also
carries a one-byte tag (a fast hash of the key) that lets setup skip most of
them, so a key opens in about one try even with a thousand. The tag is
something somebody guessing keys offline can use to rule candidates out, which
only helps against keys that are easy to guess, so a file with fewer keys has
no tags at all: use `bach key`'s 100-bit keys. Setup files made before slots
(one key for the whole file) still open.

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
| `repair_button` | Repair |
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
| `downgrade_title`, `downgrade_subtitle` (`{old_version}`), `downgrade_button` | Installing over a newer version. `downgrade_button` also confirms the next one. |
| `error_unsafe_folder` | A machine-wide app's install folder that ordinary users can change. |
| `overwrite_title`, `overwrite_subtitle` (`{files}`) | A first install into a folder that already holds files the install would replace: the window names up to five of them (and how many more) before it installs. |
| `close_app`, `error_close_app` | "Close and continue", and what it says when the app won't close. |
| `support_save`, `support_saved`, `support_failed` | The failed page's button "Save support info", what it says once the file is on the desktop, and when it couldn't be saved. The button writes `<app id>-support.txt` to the desktop: the app's name, version and id, Bach's version, the Windows build, the processor type, whether setup is elevated, the install folder, the free space, the failure's sentence and setup's log, with the user's name and profile folder replaced and no key and no app files. Nothing is sent. |
| `requirement_title`, `requirement_subtitle`, `requirement_button` (all `{requirement}`), `requirement_skip` | The page for a missing requirement, and its "Continue anyway" button. |
| `step_downloading`, `error_download` (`{error}`) | Updating from the app. |
| `step_finding`, `step_fetching`, `error_fetch` (`{error}`) | The online installer's labels, and what it says when it can't download. |
| `component_download` (`{size}`) | The line under a component whose files are downloaded when its box is ticked, like "120.0 MB download". Also the label while they download is `step_fetching`, with the component's title for `{name}`. |
| `access_title`, `access_subtitle`, `access_hint`, `access_button`, `access_paste`, `access_get`, `access_wrong` | The access key page. |
| `step_running`, `error_run` (`{task}`, `{code}`) | A `[[run]]` step's label and its failure. |
| `step_migrating` | The label while an older install by another installer is removed (`[[migrate]]`). |
| `error_patch_base` | What an update patch says when the installed files aren't the version it was made for. |
| `website`, `feedback` | The link buttons on the last page. |

`crates/pack/src/i18n/en.rs` has every English text in full, and the other
files there every translation.

## [uninstall]

| Setting | Default | Meaning |
| --- | --- | --- |
| `data` | none | Folders the app keeps its data in. The uninstaller offers "Also delete my data" (unticked) and deletes them only when the user ticks it. Each must start with `%APPDATA%`, `%LOCALAPPDATA%` or `%USERPROFILE%`, followed by your app's own folder, so the uninstaller can only ever delete inside the user's profile. A profile folder itself (`Documents`, `Desktop`, `Downloads`, `Pictures`, `Music`, `Videos`, `OneDrive`, `AppData`, `AppData\Roaming`, `AppData\Local`, `Programs`, `Temp`) is refused, and so is anything inside `.ssh`, `.aws`, `.azure`, `.gnupg`, `.kube`, `.docker`, `.terraform` or a `Microsoft` folder, a tool or work folder directly in the profile (`.config`, `.local`, `.git`, `.vscode`, `.npm`, `.cargo`, `.rustup`, `.nuget`, `.m2`, `.gradle`, `source`, `repos`, `projects`, `code`, `workspace`, `src`), another program's data (`Google\Chrome`, `Mozilla\Firefox`, `Code`, `Discord`, `Slack`, `Dropbox` and the like), a company's whole folder (`Google`, `Mozilla`, `Adobe`, `JetBrains`, `NVIDIA`, ...) and `NTUSER.DAT` or `UsrClass.dat`; a folder inside one of the shared ones (`%USERPROFILE%\Documents\My Game`) is fine. `bach check` warns when none of a data folder's parts looks like your app's name, id or publisher. The uninstaller shows the real folders, as they are on that computer, under the "Also delete my data" checkbox. `bach check` names the setting and the reason, and setup files that break the rule are refused when they start. The uninstaller also never deletes a folder that is, or contains, the install folder. |
| `data_description` | a generic line | What lives in those folders, in a few words, shown under the checkbox. Text or `{ en = "…", de = "…" }`. |
| `feedback_url` | none | An `https` page the uninstaller's last page links to ("Tell us why"). |

Write Windows paths in TOML with single quotes so backslashes stay as they
are: `data = ['%APPDATA%\My App']`.

## [profiles]

A profile is another build of the same app from the same `bach.toml`: a normal
setup file, a company version that installs for every user, a portable ZIP.
Each profile holds only what differs from the file above it.

```toml
[profiles.enterprise]
description = "For IT departments: machine-wide, no shortcuts, no self-updates."

[profiles.enterprise.installer]
scope = "machine"
desktop_shortcut = false
start_with_windows = false

[profiles.portable]
kind = "zip"
description = "Unzip and run; nothing is installed."

# Replaces [[components]]: the portable build carries the samples.
[[profiles.portable.components]]
id = "samples"
title = "Samples"
files = ["samples/**"]
default = true
```

```bash
bach profiles                       # lists them and the files they write
bach build                          # the main setup file, as before
bach build --profile enterprise     # only that profile (repeat --profile for several)
bach build --all-profiles           # the main setup file and every profile
bach check --profile portable       # read one profile without building it
bach preview --profile enterprise   # open its installer window
```

| Setting | Meaning |
| --- | --- |
| `kind` | `"setup"` (default) writes a setup file. `"zip"` writes a ZIP of the files instead: no runtime is needed, and nothing is signed or listed in the update feed. |
| `description` | One line about what the profile is for. `bach profiles` shows it. |
| `output` | Where the file goes, with `{name}`, `{version}` and `{id}` filled in. Without it a setup file is `dist/{name}-NAME-Setup-{version}.exe` and a ZIP `dist/{name}-NAME-{version}.zip`. A setup file's name must end in `.exe`, a ZIP's in `.zip`. |

Anything else in a profile is a section of `bach.toml` (`app`, `files`,
`installer`, `components`, `registry`, `style`, ...) and is laid over the
main file:

- A **table** is merged key by key: `[profiles.enterprise.installer]` with
  `scope = "machine"` changes that one setting and leaves the rest of
  `[installer]` as it is.
- An **array** (`[[components]]`, `[[registry]]`, ...) is replaced as a whole,
  so a profile that lists `[[components]]` drops the main file's.
- A profile can't change `config_version` and can't contain profiles. A typo
  is one sentence naming it: `profiles.enterprise.scoop isn't a setting a
  profile knows`.

A profile's name is lowercase letters, digits and `-`, up to 32 characters,
because it ends up in file names.

**A ZIP** holds the files of the components ticked by default, laid out as
they are in the install folder (`hello.exe`, `sub/data.txt`). The entries have
a fixed date, so the same files give the same ZIP wherever it is built. A ZIP
can't be locked with [`[access]`](#access) (the build says so) and has no
window to preview. A single file of 4 GiB or more is stored in ZIP64.

**A setup profile** is a full setup file. Two things to know:

- It shares `app.id` with the main build unless the profile sets another. Two
  setup files with one id update and uninstall each other, which is right for
  "the same app, installed for another scope" and wrong for "both on one
  computer". Give a profile that must live next to the main build its own
  `app.id` and `installer.folder`.
- The update feed (`latest.json`) describes the main setup file only, so
  profile builds never write it. A company that installs the enterprise file
  by hand or with Intune updates by installing the next one.

`--json` adds `builds`: one object per build with `profile` (null for the main
one), `kind`, `file` and `size`. `bach profiles --json` prints `profiles`
(`name`, `kind`, `description`, `output`, `problem`).

## [arch.arm64]

One download for x64 and ARM64. The main build packs `files.source` with the
x64 setup runtime. `[arch.arm64]` names the ARM64 build of the same app, and
`bach build` then also writes an ARM64 setup file, built on the ARM64 runtime,
and lists both in the update feed.

```toml
[files]
source = "build/x64"

[arch.arm64]
source = "build/arm64"                   # the ARM64 build of the app's files
exclude = ["*.pdb"]                      # optional: files.exclude for this folder
runtime = "tools/arm64/bach-setup.exe"   # optional: or --runtime-arm64 / BACH_RUNTIME_ARM64
```

```bash
bach build --runtime-arm64 path/to/arm64/bach-setup.exe
```

```console
Building Hello 1.4.0
...
Wrote dist/Hello-Setup-1.4.0.exe (11.2 MB)

Building Hello 1.4.0 for arm64
...
Wrote dist/Hello-Setup-1.4.0-arm64.exe (10.8 MB)
  Update feed  dist/latest.json
```

| Setting | Meaning |
| --- | --- |
| `source` | The folder with the ARM64 files, relative to `bach.toml`. Required. |
| `exclude` | `files.exclude` for this folder. The main `files.exclude` applies when left out. |
| `runtime` | The ARM64 `bach-setup.exe`, relative to `bach.toml`. Left out, `--runtime-arm64 FILE` (or `BACH_RUNTIME_ARM64`) names it. |

- **The ARM64 runtime** comes from the ARM64 release: every Bach release has a
  `bach-<version>-windows-arm64.zip` with an ARM64 `bach.exe` and
  `bach-setup.exe`. It ends up inside the ARM64 setup file, so Bach checks
  that the file is an ARM64 program and refuses an x64 one. Without it the
  build stops and says so; `--x64-only` builds only the x64 file.
- **Same app, same version.** Both setup files carry the same `app.id` and
  version, so an update of either replaces the other, and `bach check` tells
  you when `arch.arm64.source` isn't a folder.
- **The file is named** like the x64 one with the architecture before the
  extension: `Hello-Setup-1.4.0-arm64.exe`. It is signed with `--sign`
  like the others.
- **The update feed** (`latest.json`) keeps the x64 setup file as its main entry
  and adds the ARM64 one under `arch.arm64` with its own link, SHA-256, size
  and, with `[updates] public_key`, its own signature. An older app that doesn't
  know `arch` ignores it and keeps taking the x64 file. See
  [Updates](updates.md#one-feed-for-x64-and-arm64).
- **The online installer** is built once, from the x64 runtime, and runs on
  both. It asks Windows what the computer is, not what it is itself, so on
  Windows on ARM it downloads the ARM64 setup file.
- **Updates** of an installed app use the architecture the app was installed
  with: an ARM64 install takes the ARM64 file, and an x64 install on an ARM
  computer stays x64.
- `bach feed SETUP.exe --arm64 SETUP-arm64.exe [--arm64-url URL]` writes the
  feed with the ARM64 listing when you build the files separately, and checks
  that it is a full ARM64 setup file of the same app and version.
- Profiles: a profile build also writes its ARM64 file.

`--json` adds `setup_arm64`, the ARM64 setup file's path.

## [signing]

Optional. Says how to sign the setup files, so `bach build --sign` can do it
instead of you running a signing tool. Without this section nothing is signed.

```toml
[signing]
program = "signtool"      # default
arguments = 'sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a'
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `program` | `signtool` | The signing program, started without a shell. `signtool` is part of the Windows SDK and is rarely on `PATH`: when it isn't, Bach uses the newest one in `C:\Program Files (x86)\Windows Kits\10\bin`. Give a full path to use another. |
| `arguments` | required | Its arguments, split like a command line. For `signtool` they start with the word `sign`. The setup file is added last; write `{file}` to put it elsewhere. `{env:NAME}` is replaced by an environment variable, so secrets stay out of `bach.toml` (the build fails when it isn't set). |

Signing runs only when you ask: `bach build --sign` (or `BACH_SIGN=1` in CI)
signs what it writes, and `bach sign FILE` signs a file you built earlier. A
`bach.toml` never starts a program by itself. Signing happens before the
update feed is written. Details,
and why the installed `uninstall.exe` stays unsigned, are in
[signing](signing.md).

With this section, `bach check` and `bach build` also warn when programs or
libraries inside the setup file aren't signed (`1 of the 2 programs and
libraries in the setup file aren't signed (helper.exe)`): signing the setup file
doesn't sign them, and Smart App Control can block an unsigned program. Sign
them in your own build.

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

```powershell
$p = Start-Process .\Hello-Setup-1.4.0.exe -ArgumentList '--silent' -Wait -PassThru
$p.ExitCode        # 0: done
```

Exit codes of a setup file and its uninstaller (a test keeps them from
changing): `0` done, or nothing to do; `1` failed (a missing requirement, a
refused downgrade or overwrite, the app is open, a full disk, a file that
couldn't be written, a failed step, a declined administrator prompt; the reason
is printed and logged); `2` a mistake in the command line or a missing access
key; `10` from `--check-update` and `--download-update`: a newer version is
available.

A setup file is a windowed program, so a shell doesn't wait for it: running
`.\Hello-Setup-1.4.0.exe --silent` directly installs but returns at once,
without an exit code. `Start-Process -Wait -PassThru` waits and hands you the
exit code. In cmd, use `start /wait "" Hello-Setup-1.4.0.exe --silent` and read
`%ERRORLEVEL%`. PowerShell also needs the `.\` in front of a file in the
current folder.

More flags go in the argument list, one text each; a path with spaces gets its
own quotes:

```powershell
Start-Process .\Hello-Setup-1.4.0.exe -Wait -PassThru -ArgumentList '--silent', '--dir', '"D:\My Apps\Hello"', '--with', 'samples,path'
```

A value can follow the flag after a space or be attached with an equals sign:
`--dir "D:\My Apps\Hello"` and `--dir="D:\My Apps\Hello"` are the same, and
every flag also works with a slash (`/DIR=D:\Apps\Hello`). Flag names ignore
case. `--with` and `--without` can be given several times and add up; another
flag given twice keeps its last value.

A mistake in the command line is never guessed at: an unknown flag, a stray
word, a flag that needs a value and gets none (or gets another flag, as in
`--dir --silent`), a value on a flag that takes none, a `--theme` that isn't
`light`, `dark` or `system` and a `--wait-for` that isn't a number all stop
setup with exit code 2 and a sentence that names the flag, before anything is
installed. A terminal gets the sentence on its error output; a double-clicked
file gets a message box; a service or deployment tool (session 0) only gets
the exit code. `--help` always works, whatever else is on the line.

| Flag | Meaning |
| --- | --- |
| `--silent` (also `/S`, `/quiet`) | No window. Exit code 0 when done, 1 when it failed (the reason is printed in a terminal), 2 for a command-line mistake. |
| `--dir PATH` | Install here instead of the default folder (a first install, when `change_folder` allows it). |
| `--with a,b` / `--without a,b` | Tick or untick options: `desktop-shortcut`, `start-menu`, `autostart`, `path`, `auto-update`, and component ids. Only options the install page would offer. |
| `--lang CODE` | The language for messages, instead of Windows' own. |
| `--theme light\|dark\|system` | The window's theme for this run, instead of `installer.theme`. |
| `--preview` | Shows the window and pretends every step: nothing is installed or removed. `bach preview` runs a setup file this way. |
| `--style NAME` | With `--preview`: shows another built-in [style](#style). A setup file otherwise always wears the style it was built with, so without `--preview` it is a mistake (exit code 2). |
| `--layout NAME` | With `--preview`: shows another built-in [layout](#layout) in place of the setup file's `[layout]`. Without `--preview` it is a mistake too. |
| `--allow-downgrade` | Install over a newer version (refused otherwise, with exit code 1). |
| `--skip-requirements` | A silent install goes on although a [`[[requirements]]`](#requirements) entry isn't detected (it stops with exit code 1 otherwise). Setup says which ones, in the terminal and in the log, and that the app may not start without them. Use it when the detection is wrong for a computer, for example a .NET runtime on an unusual drive. |
| `--repair` | With `--silent`, run the setup file of the version that is installed with this flag, and setup checks every file against the SHA-256 in its manifest and writes back the ones that are missing or changed, then redoes shortcuts and registration. Nothing else in the folder and no user data is touched. The window offers the same as a **Repair** button next to **Reinstall** when the installed version is the setup file's own, and repairs with the options the app was installed with. A different version, or nothing installed, stops with exit code 1 and says so. The log lists what was written. |
| `--overwrite` | A first install into a folder that already holds files of the same names as the setup file's (and no earlier install of this app) is refused with exit code 1, naming up to five of them; this installs over them anyway. |
| `--key-file FILE` | The access key (first line of the file) for a setup file with [`[access]`](#access). The `BACH_ACCESS_KEY` environment variable does the same. Without a key a silent run exits with code 2. |
| `--log FILE` | Where the log goes. By default every run writes `%TEMP%\<app id>-setup.log`; a space in the app id is written `%20` in file and folder names (`My App` gives `My%20App-setup.log`), so two ids never share one. |
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

```powershell
$p = Start-Process "$env:LOCALAPPDATA\Programs\Hello\uninstall.exe" -ArgumentList '--check-update' -Wait -PassThru
$p.ExitCode        # 10: a newer version is out, 0: up to date
```

Use `Start-Process -Wait -PassThru` as in [silent installs](#silent-installs):
`uninstall.exe` is a windowed program too, and a plain call doesn't wait or
give you its exit code. From your own app, start it with your language's
process API and wait for it.

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
| `--channel beta\|stable\|show` | Follow the beta or the stable update feed from now on, or say which one this is. Needs `[updates] beta_feed` for `beta`. |

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

## What compressing costs

Measured with a Release build on 300 MB of mixed files (half random, half
text), 12 cores; the setup file's size does not change with the thread count:

| Level | Threads | Peak memory | Time |
| --- | --- | --- | --- |
| best (zstd 19) | 1 / 4 / 12 | 291 / 714 / 961 MB | 172 / 76 / 68 s |
| normal (zstd 9) | 1 / 4 / 12 | 156 / 299 / 443 MB | 2.1 / 1.4 / 1.5 s |
| fast (zstd 3) | 1 / 4 / 12 | 83 / 140 / 268 MB | 1.2 / 1.1 / 1.1 s |

`best` takes the most memory, so by default a build keeps it under
`compression_memory` (1024 MB) by using fewer threads: 6 of 12 here, with a
peak of 880 MB. In a container with a memory limit, set `compression_memory`
below it, or `compression_threads = 1`.

## Listing your app in winget

```bash
bach winget dist/Hello-Setup-1.4.0.exe --url https://example.com/Hello-Setup-1.4.0.exe --license MIT
```

writes the three manifests the Windows Package Manager needs; check them
with `winget validate` and open a pull request to microsoft/winget-pkgs.
CI checks them with `winget validate` (`scripts/test-winget.ps1`), also for an
app whose id is 50 characters long. They name the oldest supported Windows
(`MinimumOSVersion: 10.0.17763.0`) and quote every link, so a `#` in it stays
part of it. The manifests say `Scope: machine` for a setup file with
`installer.scope = "machine"` (otherwise `user`), and list it under the
architecture of its runtime: `x64`, or `arm64` for a setup file built with the
ARM64 runtime.

Bach's own release workflow opens the winget pull request for each normal
release with `wingetcreate` (never for a pre-release, a draft or a dry run),
when the repository has the secret `WINGET_TOKEN` and the variable
`WINGET_PACKAGE`. The package has to be listed already: the first pull request
is made by hand.

## Listing your app in Chocolatey

```bash
bach choco dist/Hello-Setup-1.4.0.exe --url https://example.com/Hello-Setup-1.4.0.exe --license MIT
```

writes a Chocolatey package into `choco/<id>/<version>` next to the setup
file: `<id>.nuspec`, `tools/chocolateyInstall.ps1` (downloads the file, checks
its SHA-256, runs it with `--silent`) and `tools/chocolateyUninstall.ps1` (runs
the app's own uninstaller from its Settings › Apps entry). The id is the app
id in lowercase. Build it with `choco pack` in that folder. Chocolatey runs as
an administrator, so a per-user app is installed for that account only; the
command warns, and `installer.scope = "machine"` fixes it. `--license` takes a
link or a name like `MIT`; `--out` picks the folder.

## Deploying with Intune

Companies hand out apps with Microsoft Intune, Configuration Manager or a
similar tool. For a setup file that isn't an MSI they ask for three things:
a silent install command, an uninstall command and a rule that says the app
is installed. Bach has all three already; one command writes them down:

```bash
bach intune dist/Hello-Setup-1.4.0.exe
```

```console
Wrote the Intune settings for Hello 1.4.0 to dist\intune\Hello
  Install      Hello-Setup-1.4.0.exe --silent
  Uninstall    "%ProgramFiles%\Hello\uninstall.exe" --silent
  Run as       System (installs for every user)
  Detect       registry HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Hello, DisplayVersion >= 1.4.0
  Requires     64-bit (x64), Windows 10 1809 or newer
```

It writes two files into `intune/<app id>` next to the setup file (`--out`
picks another folder):

| File | What it holds |
| --- | --- |
| `intune.md` | The settings sheet: how to wrap the setup file with Microsoft's Win32 Content Prep Tool (`IntuneWinAppUtil.exe`), the program settings, the requirements and the detection rule, ready to copy into Intune. |
| `detect.ps1` | The same detection as a custom detection script. It reads the app's Settings › Apps entry and says "installed" when this version **or a newer one** is there, so Intune never installs an old version over one Bach has updated. Leave "Run script as 32-bit process" off. |

Things to know:

- A machine-wide app (`installer.scope = "machine"`) is the one for Intune
  devices: Intune runs setup as the system account, so nothing asks for
  permission, and the registry rule works. A per-user app is deployed with
  install behavior **User** and detected with `detect.ps1` (the command warns).
- The commands use the default install folder. If you install somewhere else
  with `--dir`, point the uninstall command there too.
- A version like `2.0.0-beta.1` isn't made of plain numbers, so Intune's
  registry rule can't compare it; the sheet then says to use `detect.ps1`.
- A setup file with [`[access]`](#access) needs its key: add `--key-file FILE`
  to the install command and put the key file in the package.
- Keep the app id and the scope the same in every release. Windows tells apps
  apart by them, and a changed scope installs next to the old app.
- The exit codes are the ones in [silent installs](#silent-installs): `0` is
  success and `1` a failure, which are Intune's defaults too.

`--json` prints `install_command`, `uninstall_command`, `run_as`,
`detection_key` (machine-wide apps) and `detection_script`.

## Windows Installer packages (MSI)

Companies that deploy with Group Policy, Configuration Manager or Intune ask
for an `.msi`. `bach build --msi` writes one from the same `bach.toml`, next to
the setup file:

```bash
bach build --msi
```

```console
Wrote dist/Hello-Setup-1.4.0.exe (11.2 MB)
Wrote dist/Hello-Setup-1.4.0.msi (9.8 MB), the Windows Installer package
  not in the MSI: [[run]] steps
  not in the MSI: adding to PATH
```

The package is made by Bach (a real MSI database with its files in an embedded
cabinet), not a wrapper that runs the setup file. Windows Installer installs,
repairs, upgrades and removes it itself, so deployment tools can track it.

| In the MSI | How |
| --- | --- |
| Files and folders | Every file is its own component, with an id made from the app id and the file's path, so it stays the same from one version to the next. |
| Shortcuts | The Start menu shortcut (in the folder `installer.start_menu_folder` names, which removal takes away again), the desktop one and `[[shortcuts]]`, each in a component of its own, with the app's icon. |
| Registry values, variables and integration | `[[registry]]`, `[[env]]` variables, `[[protocols]]`, `[[file_types]]` (as "Open with") and `[[context_menu]]` entries, written and removed exactly where the setup file writes them. `{app}` is the install folder, here with a trailing backslash. |
| Optional components and options | One feature each, named like the setup file's own `--with` ids: a component by its id, and the options of the install page as `desktop_shortcut`, `start_menu`, `autostart` and `path` (a `-` is written `_`). Ticked ones install by default; the others with `ADDLOCAL=id`. An option that is `always` is part of the product, `never` isn't in the package. |
| Start with Windows, PATH | The `Run` entry, and the install folder (or `installer.path_folder`) appended to the user's `PATH` and taken off again on removal, the rest of `PATH` kept. |
| Run steps | `[[run]]` steps are custom actions that start the packaged program after the files are in place (`install`, `first_install`) or before they go (`uninstall`, never stopping the removal), for the component they name. |
| Requirements | A `windows` requirement is a launch condition on the Windows build. |
| Settings › Apps | Name, publisher, version, website, icon and the install location. |
| Upgrades | A package of a higher version replaces older ones (a major upgrade; the upgrade code comes from `app.id`). An older package refuses to replace a newer one (error 1603, with "A newer version of Hello is already installed."). |
| Scope | `installer.scope = "machine"`: per machine, in `Program Files`, for all users. The default `"user"`: per user, in `%LOCALAPPDATA%\Programs`, and installing needs no administrator rights. |

What it doesn't carry, and says so when your config uses it (`not in the MSI:`):
`[[migrate]]`, `[updates]` (an MSI is updated by installing a newer MSI),
`[uninstall]` data folders, the license page (an MSI has no window of its own here),
the `download` of a component (its files are in the package), requirements
other than a Windows build (WebView2, .NET, VC++, file and registry checks), a
run step's other exit codes (Windows Installer accepts `0`) and an `elevated` run
step in a per-user package (it never has administrator rights). Use the setup
file where you need those. Windows texts are English.

```powershell
msiexec /i Hello-Setup-1.4.0.msi /qn                    # install silently
msiexec /i Hello-Setup-1.4.0.msi /qn ADDLOCAL=ALL       # with every optional component
msiexec /x Hello-Setup-1.4.0.msi /qn                    # remove it
```

Limits and rules:

- **The version** has to be three numbers (`1.4.2`): the first two up to 255, the
  last up to 65535, and no suffix like `-beta`. Windows Installer compares
  versions that way. Anything else is refused, naming `app.version`.
- **Size.** The cabinet holds up to 2 GB and 32,000 files; a bigger app is refused with that
  sentence. Names (the app's, the publisher's, the folders') are written in the
  Windows-1252 code page that Windows Installer reads, so a character outside it shows as `?`.
- **An app locked with [`[access]`](#access)** can't be an MSI: the files in the
  package would be readable by anybody. The build refuses it.
- The package is **x64**: the ARM64 setup file (`[arch.arm64]`) isn't turned into an MSI. `--msi` with a
  [profile](#profiles) writes that profile's package next to its setup file (`Hello-enterprise-Setup-1.4.0.msi`),
  which is how a company version gets its own; a `zip` profile has none.
- It isn't signed by `--sign` (Windows Installer's own signing is a different
  mechanism): sign it with `signtool sign` like any other file.
- `--json` adds `msi` (the file) and `msi_skipped` (what it left out).
- A real run: Bach's own tests install a package like this per user with every item
  above, check the registry, `PATH`, the variables, the shortcuts and the run steps, upgrade it,
  see an older package refused and remove it again with `msiexec`, checking that all of it goes.

## Checking a setup file

```bash
bach verify dist/Hello-Setup-1.4.0.exe
```

```console
Verifying Hello 1.4.0 (dist/Hello-Setup-1.4.0.exe)
  Signature    not signed
  Archive      SHA-256 matches (8650123 bytes)
  Files        12 checked against the manifest
dist/Hello-Setup-1.4.0.exe is intact.
```

Use it on a download that may have gone wrong, a copy someone changed, or the
file a build server hands over. It installs and writes nothing. It reads the
archive and checks what setup checks while it installs: the archive against the
SHA-256 recorded when the setup file was built, and every file against the
size and SHA-256 in the manifest. It also looks for files the manifest lacks
or the archive doesn't hold. A damaged file stops with exit code 1 and a
sentence that names the first few files that are wrong.

- `signature` says whether the file carries a signature. Bach reads only the
  entry in the file's header; Windows decides whether it is valid, and
  `signtool verify /pa` checks it (see [signing](signing.md)).
- The runtime part of the file (the program that shows the window) is covered
  by that signature, not by `bach verify`.
- A setup file with [`[access]`](#access) has its files encrypted. Without a
  key `bach verify` still checks the archive as a whole and says the files stayed
  shut; give the key with `--key-file FILE` (or `BACH_ACCESS_KEY`) to open
  them and check each one. A wrong key is refused, not reported as damage.
- An online installer and an uninstaller carry no files, so there is nothing
  more to check. An update patch is checked too; the files it leaves out are on
  the computer already.

`--json` prints `verified`, `signature`, `files`, `name` and `version`; a
damaged file is the usual `{"ok": false, "error": "…"}`.

## Comparing two setup files

`bach diff` reads two setup files of one app and says what changed, without
unpacking anything. Use it to see what a release really ships, or why it got
bigger.

```bash
bach diff dist/Hello-Setup-1.0.0.exe dist/Hello-Setup-1.1.0.exe
```

```console
Hello 1.0.0 → 1.1.0
  Files        1 added, 1 removed, 1 changed, 0 unchanged
  Installed    585.9 KB → 195.3 KB (-390.6 KB)
  Packed       586.0 KB → 195.4 KB (-390.6 KB)

Files:
  - data.bin  (585.9 KB)
  + extra.dat  (195.3 KB)
  ~ hello.exe  (2 bytes → 4 bytes, +2 bytes)

Made the app bigger:
     +195.3 KB  extra.dat
      +2 bytes  hello.exe
```

- A file counts as **changed** when its SHA-256 differs (a setup file built
  before checksums were recorded is compared by size, and the output says so).
  Names are matched the way Windows matches them, so `Data.bin` and `data.bin`
  are one file.
- **Besides files** lists what the new setup file does to Windows that the old
  one didn't (`+`), no longer does (`-`) or does differently (`~`):
  `[[registry]]` values, `[[env]]` variables, `[[context_menu]]` entries,
  `[[shortcuts]]`, link schemes, file types, `[[run]]` steps, requirements,
  components and `[[migrate]]` entries.
- **Made the app bigger** ranks the files by how many bytes they added to the
  installed app, biggest first (`--top N`, default 5). The archive is one
  compressed stream, so Bach can't say how much each file added to the
  *setup file*; the installed size and the packed total are shown instead.
- Both files must be for the same app id. An installed `uninstall.exe` works
  as either side too.

`--json` prints `old_version`, `new_version`, `unchanged`, `installed_growth`,
`setup_size_old`, `setup_size_new`, `changed_files` (`path`, `change`,
`old_size`, `new_size`) and `changes` (`change`, `what`).

## Testing a setup file in Windows Sandbox

`bach test` installs your setup file on a clean Windows, starts the app, removes
it and tells you what went wrong and what was left behind. The sandbox is a
fresh copy of Windows every time and is thrown away when the test ends, so
nothing is installed on your computer.

```bash
bach build
bach test                          # the file bach.toml builds
bach test dist/Hello-Setup-1.4.0.exe --with samples
```

```console
Testing Hello 1.4.0 in Windows Sandbox
  Sandbox      starting; a fresh Windows takes about a minute
  ok    Installed      exit code 0 in 0.7 s
  ok    Registered     Settings > Apps lists Hello 1.4.0
  ok    Files          12 of 12 files are in C:\Users\WDAGUtilityAccount\AppData\Local\Programs\Hello
  ok    Uninstaller    uninstall.exe is in the install folder
  ok    Started        still running after 5 s
  ok    Removed        exit code 0 in 0.7 s
  FAIL  Left behind    1 files and folders, 0 registry keys, 0 other changes

Files and folders left behind:
  C:\Users\WDAGUtilityAccount\AppData\Local\Hello
```

What it checks, in order:

1. **Installed**: the setup file's silent install (`--silent`) ends with exit code 0.
2. **Registered**: Settings › Apps lists the app with the setup file's version.
3. **Files**: every file the install should have written is there, with the size
   the manifest records.
4. **Uninstaller**: `uninstall.exe` is in the install folder.
5. **Started**: the app starts and, in 5 seconds (`--launch SECONDS`; `0` skips
   this), does not end with an error. An app that is still running counts as
   good; one that ends with exit code 0 does too. The test closes it afterwards.
6. **Removed**: the silent removal (`uninstall.exe --silent --delete-data`) ends
   with exit code 0 and takes the Settings › Apps entry away.
7. **Left behind**: it compares the folders an install writes to (your profile,
   Program Files, ProgramData, the desktop and the Start menu, two levels deep),
   the registry (`Software`, the uninstall entries), user environment variables,
   the Run key and scheduled tasks with how they were before the install. What is
   new afterwards is listed, and the check fails. Windows' own folders
   (`Temp`, `Microsoft`, `Packages`, ...) are left out because an app doesn't
   write there.

The report ends with a summary line and exit code `0` when every check passed,
`1` otherwise.

| Option | Meaning |
| --- | --- |
| `FILE` | The setup file to test. Without it, the file `bach.toml` builds (`bach build` first). |
| `--with IDS`, `--without IDS` | Options to install with, like the setup file's own `--with` (`samples`, `desktop-shortcut`, ...). A component's files are only expected when it is on. |
| `--launch SECONDS` | How long to leave the app running (default 5; `0` doesn't start it). |
| `--network` | Give the sandbox the network. It is off by default, so the test needs nothing from outside; an online installer needs it (and says so). |
| `--timeout MINUTES` | How long to wait for the report (default 20). |
| `--keep` | Leave the sandbox open when the test ends, to look around in it. |
| `--dry-run` | Write the files the test would use and stop. It works without Windows Sandbox and prints where the `.wsb` file is. |

`--json` prints `passed`, `steps` (`name`, `ok`, `detail`) and `left_behind`
(`files`, `registry`, `other`).

- **Windows Sandbox** comes with Windows 10 and 11 **Pro, Enterprise and
  Education** (not Home). Turn it on under "Turn Windows features on or off",
  restart, and `bach test` finds it. Only one sandbox runs at a time.
- A setup file that asks for **administrator rights** (`installer.scope =
  "machine"`, or an elevated run step) shows Windows' prompt inside the sandbox
  window; accept it there. A setup file with **[`[access]`](#access)** asks
  for a key a silent test can't type: test a build without it.
- The files in the sandbox's folders are the setup file, a script that Bach
  writes (`run.ps1`), and `params.json` with what the script checks against.
  The script reads that file as data and never builds commands from what a
  setup file says. The setup file's folder is mapped read-only.

## A download page

```bash
bach page dist/Hello-Setup-1.4.0.exe --url https://example.com/downloads/Hello-Setup-1.4.0.exe
```

writes `download/index.html` next to the setup file (`--out` picks the folder):
the app's name, version, publisher and size, a download button, how to install
and update, how to remove it, and the file's SHA-256 with the PowerShell
command that checks it. The numbers come from the setup file, so they can't
disagree with the file.

| Flag | Meaning |
| --- | --- |
| `--url URL` | Required. The `https` link the setup file is downloaded from. |
| `--out DIR`, `-o DIR` | The folder for `index.html`. |
| `--notes-file FILE` | What's new, up to 20 KB. Lines starting with `-` or `*` become a list; nothing else is interpreted, so Markdown and HTML show as written. |
| `--winget ID` | Shows `winget install ID` (the id looks like `Publisher.AppName`). |

The page is one file with no script and no outside file: it follows the
reader's light or dark setting, works on a phone, and has a content security
policy that allows nothing but its own style. The button uses the setup
file's `accent` colour, and a PNG logo travels inside the page (a logo over
256 KB, or an SVG one, is left out, and the page shows the name). The
install text says whether the app installs for one user or, with
`installer.scope = "machine"`, asks for administrator permission once. Put the
page where your downloads are; the text is in English.

An online installer or an uninstaller is refused: point `bach page` at the full
setup file. `--json` adds `page` (the file written), `sha256` and `size`.

## Failing on warnings

`bach check` and `bach build` print a warning for what builds but probably
isn't meant: a long file list, an elevated step in a per-user install,
unsigned programs inside a signed setup file and more. In a pipeline that is
easy to miss. Add `--strict` (or set `BACH_STRICT=1`) and any warning is an
error:

```console
$ bach check --strict
...
warning: installer.path_folder is "bin", but no installed file is in it.
error: bach.toml has 1 warning, and --strict treats it as an error: fix it, or run without --strict.
```

`bach build --strict` stops before it builds, so no file is written. Without
the flag nothing changes.
