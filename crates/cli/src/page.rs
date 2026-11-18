//! `bach page`: a download page for a setup file, as one HTML file.
//!
//! Most developers end up writing the same page: the app's name, a download
//! button, the size, the SHA-256 and how to check it, and a line on how to
//! install. This writes it from the setup file itself, so the numbers on the
//! page are the numbers of the file next to it. The page has no script and no
//! external file (the logo, when it is a PNG, travels inside it), and says so
//! to the browser in a content security policy.

use std::path::Path;

use anyhow::{bail, Context, Result};
use bach_pack::container;
use bach_pack::manifest::{LogoKind, Manifest};

use crate::output::{self, field};

pub struct Options<'a> {
    pub setup: &'a Path,
    /// The `https` link the setup file is downloaded from.
    pub url: &'a str,
    pub out: Option<&'a Path>,
    /// A text file with what's new.
    pub notes_file: Option<&'a Path>,
    /// The winget package id, to show `winget install ID`.
    pub winget: Option<&'a str>,
}

/// A logo this large would make the page heavy for no gain.
const MAX_LOGO_BYTES: usize = 256 * 1024;
/// What's new is a few lines, not a changelog.
const MAX_NOTES_BYTES: u64 = 20 * 1024;

pub fn command(options: Options) -> Result<()> {
    if !bach_pack::manifest::extras::is_https(options.url) {
        bail!("--url must be the https link the setup file is downloaded from");
    }
    if let Some(id) = options.winget {
        if !is_winget_id(id) {
            bail!("--winget is \"{id}\"; use the package id, like Publisher.AppName");
        }
    }
    let mut file = std::fs::File::open(options.setup).with_context(|| format!("couldn't open {}", options.setup.display()))?;
    let Some(layout) = container::locate(&mut file)? else {
        bail!("{} isn't a Bach setup file", options.setup.display());
    };
    let manifest = container::read_manifest(&mut file, &layout)?;
    if !layout.has_archive() {
        let what = if manifest.online { "an online installer" } else { "an uninstaller" };
        bail!("{} is {what}; point bach page at the full setup file", options.setup.display());
    }
    let (sha256, size) = crate::feed::digest_of(options.setup)?;
    let architecture = crate::winget::setup_architecture(options.setup)?;
    let mut warnings = Vec::new();
    let logo = match (&manifest.look.logo, manifest.look.hide_logo) {
        (Some(LogoKind::Png), false) => {
            let bytes = container::read_look(&mut file, &layout, &manifest)?.logo;
            if bytes.len() > MAX_LOGO_BYTES {
                warnings
                    .push(format!("the logo is {} KB, too big to carry inside the page; the page shows the name only", bytes.len() / 1024));
                None
            } else {
                Some(bytes)
            }
        }
        _ => None,
    };
    let notes = match options.notes_file {
        Some(path) => {
            let length = std::fs::metadata(path).with_context(|| format!("couldn't read {}", path.display()))?.len();
            if length > MAX_NOTES_BYTES {
                bail!("{} is {} KB; what's new on a download page is a few lines (20 KB at most)", path.display(), length / 1024);
            }
            Some(bach_pack::textfile::read(path).with_context(|| format!("couldn't read {}", path.display()))?)
        }
        None => None,
    };
    let file_name = options.setup.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "Setup.exe".into());
    let page = Page {
        manifest: &manifest,
        url: options.url,
        file_name: &file_name,
        size,
        sha256: &sha256,
        architecture,
        logo: logo.as_deref(),
        notes: notes.as_deref(),
        winget: options.winget,
    };
    let dir = match options.out {
        Some(out) => out.to_path_buf(),
        None => options.setup.parent().unwrap_or(Path::new(".")).join("download"),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let target = dir.join("index.html");
    std::fs::write(&target, page.render()).with_context(|| format!("couldn't write {}", target.display()))?;

    output::result("page", target.display().to_string());
    output::result("name", manifest.app.name.clone());
    output::result("version", manifest.app.version.clone());
    output::result("sha256", sha256.clone());
    output::result("size", size);
    say!("Wrote the download page for {} {} to {}", manifest.app.name, manifest.app.version, target.display());
    field("Download", options.url);
    field("SHA-256", &sha256);
    output::warnings(&warnings);
    say!("It is one file with no script. Put it where your downloads are, next to or instead of your own page.");
    Ok(())
}

/// What the page shows.
struct Page<'a> {
    manifest: &'a Manifest,
    url: &'a str,
    file_name: &'a str,
    size: u64,
    sha256: &'a str,
    architecture: &'static str,
    logo: Option<&'a [u8]>,
    notes: Option<&'a str>,
    winget: Option<&'a str>,
}

impl Page<'_> {
    fn render(&self) -> String {
        let app = &self.manifest.app;
        let accent = self.manifest.look.accent.as_deref().filter(|colour| is_colour(colour));
        let (button, button_ink) = match accent {
            Some(colour) => (colour.to_string(), if is_light(colour) { "#111111" } else { "#ffffff" }),
            None => ("var(--ink)".to_string(), "var(--page)"),
        };
        let machine = self.manifest.install.scope.is_machine();
        let mut body = String::new();
        if let Some(logo) = self.logo {
            body.push_str(&format!("<img class=\"logo\" src=\"data:image/png;base64,{}\" alt=\"\" height=\"56\">\n", base64(logo)));
        }
        body.push_str(&format!("<h1>{}</h1>\n", html(&app.name)));
        let mut facts = vec![format!("Version {}", html(&app.version)), html(&app.publisher), output::size(self.size)];
        facts.push(
            match self.architecture {
                "arm64" => "Windows on ARM64",
                "x86" => "Windows, 32-bit",
                _ => "Windows 64-bit",
            }
            .to_string(),
        );
        body.push_str(&format!("<p class=\"facts\">{}</p>\n", facts.join(" · ")));
        if let Some(description) = &app.description {
            body.push_str(&format!("<p class=\"lead\">{}</p>\n", html(description)));
        }
        body.push_str(&format!(
            "<p><a class=\"button\" href=\"{}\" rel=\"noopener\">Download for Windows</a></p>\n<p class=\"small\">Windows 10 (version 1809) or newer.</p>\n",
            html(self.url)
        ));
        body.push_str("<h2>Install</h2>\n<ol>\n<li>Download the file above and open it.</li>\n");
        body.push_str(&format!(
            "<li>Follow the install window. {}</li>\n",
            if machine {
                "It installs for everyone on the computer, so Windows asks for administrator permission once."
            } else {
                "It installs for you alone and doesn't ask for administrator rights."
            }
        ));
        body.push_str(&format!(
            "<li>To update later, run a newer setup file the same way. It installs over the old version and keeps your settings. To remove {}, use <strong>Settings&nbsp;›&nbsp;Apps</strong>.</li>\n</ol>\n",
            html(&app.name)
        ));
        if let Some(id) = self.winget {
            body.push_str(&format!("<p>Or with the Windows Package Manager:</p>\n<pre><code>winget install {}</code></pre>\n", html(id)));
        }
        if let Some(notes) = self.notes {
            body.push_str(&format!("<h2>What's new in {}</h2>\n{}", html(&app.version), notes_html(notes)));
        }
        body.push_str(&format!(
            "<h2>Check the download</h2>\n<p>The file's SHA-256 is:</p>\n<pre><code>{}</code></pre>\n<p>To compare it, run this in PowerShell in the folder you saved it to:</p>\n<pre><code>Get-FileHash .\\{} -Algorithm SHA256</code></pre>\n",
            html(self.sha256),
            html(&powershell_name(self.file_name))
        ));
        if let Some(website) = app.website.as_deref().filter(|site| bach_pack::manifest::extras::is_https(site)) {
            body.push_str(&format!("<p class=\"small\"><a href=\"{}\" rel=\"noopener\">{}</a></p>\n", html(website), html(website)));
        }
        TEMPLATE
            .replace("@TITLE@", &html(&format!("Download {} {}", app.name, app.version)))
            .replace("@BUTTON@", &button)
            .replace("@BUTTON_INK@", button_ink)
            .replace("@BODY@", &body)
    }
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>@TITLE@</title>
<style>
:root { --page: #ffffff; --ink: #111111; --muted: #5b5b5b; --line: #e4e4e4; --code: #f4f4f4; --button: @BUTTON@; --button-ink: @BUTTON_INK@; }
@media (prefers-color-scheme: dark) { :root { --page: #181818; --ink: #f2f2f2; --muted: #a3a3a3; --line: #2e2e2e; --code: #232323; } }
* { box-sizing: border-box; }
body { margin: 0; background: var(--page); color: var(--ink); font: 16px/1.6 system-ui, -apple-system, "Segoe UI", Inter, sans-serif; }
main { max-width: 40rem; margin: 0 auto; padding: 3rem 1rem 4rem; }
h1 { font-size: 2rem; line-height: 1.2; margin: 1rem 0 .25rem; overflow-wrap: anywhere; }
h2 { font-size: 1.15rem; margin: 2.5rem 0 .5rem; padding-top: 1.5rem; border-top: 1px solid var(--line); }
.logo { display: block; height: 56px; width: auto; max-width: 100%; }
.facts, .small { color: var(--muted); font-size: .9rem; margin: .25rem 0; }
.lead { font-size: 1.1rem; margin: 1rem 0; }
.button { display: inline-block; margin-top: 1rem; padding: .75rem 1.5rem; border-radius: .6rem; background: var(--button); color: var(--button-ink); font-weight: 600; text-decoration: none; }
.button:focus-visible, a:focus-visible { outline: 3px solid var(--ink); outline-offset: 3px; }
ol, ul { padding-left: 1.25rem; }
li { margin: .35rem 0; }
pre { margin: .5rem 0 1rem; padding: .75rem 1rem; background: var(--code); border-radius: .5rem; overflow-x: auto; }
code { font: .85rem/1.5 ui-monospace, "Cascadia Mono", Consolas, monospace; overflow-wrap: anywhere; }
a { color: inherit; }
</style>
</head>
<body>
<main>
@BODY@</main>
</body>
</html>
"#;

/// Text for an HTML body or attribute.
fn html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// What's new: lines starting with `-` or `*` become a list, the rest
/// paragraphs. Markdown headings and emphasis are not interpreted: the text
/// is shown as written, escaped.
fn notes_html(notes: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for line in notes.replace("\r\n", "\n").lines().map(str::trim).filter(|line| !line.is_empty()) {
        let item = line.strip_prefix("- ").or_else(|| line.strip_prefix("* "));
        match (item, in_list) {
            (Some(item), false) => {
                out.push_str("<ul>\n");
                out.push_str(&format!("<li>{}</li>\n", html(item.trim())));
                in_list = true;
            }
            (Some(item), true) => out.push_str(&format!("<li>{}</li>\n", html(item.trim()))),
            (None, true) => {
                out.push_str("</ul>\n");
                out.push_str(&format!("<p>{}</p>\n", html(line)));
                in_list = false;
            }
            (None, false) => out.push_str(&format!("<p>{}</p>\n", html(line))),
        }
    }
    if in_list {
        out.push_str("</ul>\n");
    }
    out
}

/// `#rrggbb`: the only colour syntax that goes into the page's CSS.
fn is_colour(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether black text reads better on this colour than white.
fn is_light(colour: &str) -> bool {
    let channel = |at: usize| u32::from_str_radix(&colour[at..at + 2], 16).unwrap_or(0) as f32;
    0.299 * channel(1) + 0.587 * channel(3) + 0.114 * channel(5) > 150.0
}

/// A winget id: `Publisher.Package`, letters, digits and dots.
fn is_winget_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.contains('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// A file name inside a PowerShell command line: quoted when it has a space.
fn powershell_name(name: &str) -> String {
    if name.contains([' ', '\'', '`', '$', '&', '(', ')', ';']) {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        name.to_string()
    }
}

/// Standard base64 with padding, for the logo inside the page.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let block = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(block >> 18) as usize & 63] as char);
        out.push(ALPHABET[(block >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(block >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[block as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(scope: &str, accent: Option<&str>) -> Manifest {
        let accent = accent.map(|colour| format!("\"{colour}\"")).unwrap_or_else(|| "null".into());
        serde_json::from_str(&format!(
            r#"{{"format":1,"builder":"t","app":{{"id":"Hello","name":"Hello <b>&</b>","version":"1.4.0","publisher":"Hello \"Inc\".",
            "description":"Says <hi>.","website":"https://hello.example/?a=1&b=2","executable":"hello.exe"}},
            "install":{{"folder":"Hello","scope":"{scope}","desktop_shortcut":"never"}},"uninstall":{{"data":[],"data_description":null}},
            "look":{{"theme":"system","accent":{accent},"logo":null}},"files":[{{"path":"hello.exe","size":1}}]}}"#
        ))
        .unwrap()
    }

    const SHA: &str = "abababababababababababababababababababababababababababababababab";

    fn page<'a>(manifest: &'a Manifest, notes: Option<&'a str>, winget: Option<&'a str>) -> Page<'a> {
        Page {
            manifest,
            url: "https://hello.example/dl?a=1&b=\"2\"",
            file_name: "Hello Setup-1.4.0.exe",
            size: 9_123_456,
            sha256: SHA,
            architecture: "x64",
            logo: None,
            notes,
            winget,
        }
    }

    #[test]
    fn everything_the_setup_file_says_is_escaped() {
        let manifest = manifest("user", Some("#2563eb"));
        let text = page(&manifest, Some("- fixed <script>alert(1)</script>\nplain & simple"), Some("Hello.App")).render();
        for danger in ["<script>", "<b>", "<hi>", "\"2\"", "Hello \"Inc\"."] {
            assert!(!text.contains(danger), "{danger} in {text}");
        }
        assert!(text.contains("Hello &lt;b&gt;&amp;&lt;/b&gt;"), "{text}");
        assert!(text.contains("href=\"https://hello.example/dl?a=1&amp;b=&quot;2&quot;\""), "{text}");
        assert!(
            text.contains("<li>fixed &lt;script&gt;alert(1)&lt;/script&gt;</li>") && text.contains("<p>plain &amp; simple</p>"),
            "{text}"
        );
        assert!(text.contains("winget install Hello.App"));
        assert!(
            text.contains("Get-FileHash .\\'Hello Setup-1.4.0.exe' -Algorithm SHA256")
                || text.contains("Get-FileHash .\\&#39;Hello Setup-1.4.0.exe&#39;"),
            "{text}"
        );
        assert!(!text.contains("<script"), "the page has no script");
        assert!(text.contains("Content-Security-Policy") && text.contains("default-src 'none'"));
    }

    #[test]
    fn the_page_tells_the_truth_about_the_install() {
        let user = page(&manifest("user", None), None, None).render();
        assert!(user.contains("doesn't ask for administrator rights"), "{user}");
        assert!(user.contains("9.1 MB") || user.contains("8.7 MB"), "{user}");
        assert!(user.contains("Windows 64-bit") && user.contains("Version 1.4.0"), "{user}");
        let machine = page(&manifest("machine", None), None, None).render();
        assert!(machine.contains("asks for administrator permission once"), "{machine}");
        assert!(!machine.contains("winget install"), "only when asked for");
        assert!(!machine.contains("What's new"), "only with notes");
    }

    #[test]
    fn only_a_real_colour_reaches_the_css() {
        assert!(is_colour("#2563eb") && is_colour("#FFFFFF"));
        for bad in ["red", "#fff", "#2563eb;}</style>", "#2563eg", "", "url(x)"] {
            assert!(!is_colour(bad), "{bad}");
        }
        // A hostile accent in a setup file falls back to the neutral button.
        let mut hostile = manifest("user", None);
        hostile.look.accent = Some("#000;} body{display:none".into());
        let text = page(&hostile, None, None).render();
        assert!(!text.contains("display:none"), "{text}");
        assert!(text.contains("--button: var(--ink)"), "{text}");
        let blue = page(&manifest("user", Some("#2563eb")), None, None).render();
        assert!(blue.contains("--button: #2563eb") && blue.contains("--button-ink: #ffffff"), "{blue}");
        let yellow = page(&manifest("user", Some("#ffd400")), None, None).render();
        assert!(yellow.contains("--button-ink: #111111"), "{yellow}");
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xFF, 0xFE, 0x00]), "//4A");
    }

    #[test]
    fn winget_ids_and_file_names_are_checked_before_they_are_shown() {
        assert!(is_winget_id("Publisher.App") && is_winget_id("A.B.C-d_e"));
        for bad in ["", "NoDot", "a b.c", "a.b;calc", "a.b\"<"] {
            assert!(!is_winget_id(bad), "{bad}");
        }
        assert_eq!(powershell_name("Setup.exe"), "Setup.exe");
        assert_eq!(powershell_name("My App's Setup.exe"), "'My App''s Setup.exe'");
    }
}
