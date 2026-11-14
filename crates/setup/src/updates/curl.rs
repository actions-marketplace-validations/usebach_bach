//! Downloads with Windows' own `curl.exe` (never one found on `PATH`), https
//! only, redirects included. Windows 10 1803 and later ship it, so the
//! runtime carries no network code of its own.
//!
//! An app with an access key (`[access] send_key`) sends it as
//! `Authorization: Bearer <key>`. That secret never goes on a command line,
//! where other programs could read it: curl is handed it on its standard
//! input as a config file. Older curl builds also pass such a header on to
//! whatever host a redirect points at, so with a key the redirects are
//! followed here instead, and the key goes only to the host it was meant
//! for (see [`resolve`]).
//!
//! "The host it was meant for" is the host of the app's main update feed,
//! which is in the signed setup file's manifest. A feed answer can name any
//! download link, so the key never goes to a host just because a feed said
//! so: a download that lives elsewhere is fetched without it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How one download behaves.
#[derive(Clone, Copy, Debug)]
pub struct Fetch {
    /// Continue a partial file instead of starting over.
    pub resume: bool,
    /// Give up after this many seconds in all.
    pub max_seconds: u32,
    /// Refuse a response bigger than this many bytes (the feed is tiny).
    pub max_bytes: Option<u64>,
}

impl Fetch {
    /// The small JSON feed: quick, and never more than a megabyte.
    pub const FEED: Fetch = Fetch { resume: false, max_seconds: 60, max_bytes: Some(1 << 20) };
    /// A setup file: as long as it takes, resuming where an earlier try stopped.
    pub const SETUP: Fetch = Fetch { resume: true, max_seconds: 3 * 3600, max_bytes: None };
}

/// An access key and the one origin (`host` or `host:port`) it may be sent to.
pub struct Bearer {
    key: String,
    origin: String,
}

impl std::fmt::Debug for Bearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bearer(.. for {})", self.origin)
    }
}

impl Bearer {
    /// The key, tied to the origin of `feed` (the app's main update feed).
    /// None when the feed link isn't one a key may be sent to.
    pub fn for_feed(key: &str, feed: &str) -> Option<Self> {
        Some(Self { key: key.to_string(), origin: origin(feed).ok()? })
    }
}

/// What asking a link gives: its HTTP status and where it redirects (empty
/// when it doesn't).
type Answer = Result<(u16, String), String>;

/// The most redirects followed by hand when a key is sent.
const MAX_HOPS: usize = 5;

fn curl_path() -> Result<PathBuf, String> {
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let curl = system.join("System32").join("curl.exe");
    if curl.is_file() {
        Ok(curl)
    } else {
        Err("this Windows has no curl.exe (Windows 10 1803 or later has it)".into())
    }
}

/// The arguments for one download, apart from the file and the link.
/// `follow` lets curl follow redirects itself (https only).
fn arguments(how: Fetch, follow: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--fail", "--silent", "--show-error", "--proto", "=https",
        "--retry", "3", "--retry-delay", "2", "--connect-timeout", "20", "--header", "Cache-Control: no-cache",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    if follow {
        args.extend(["--location".to_string(), "--proto-redir".to_string(), "=https".to_string()]);
    }
    args.push("--max-time".into());
    args.push(how.max_seconds.to_string());
    if how.resume {
        args.extend(["--continue-at".to_string(), "-".to_string()]);
    }
    if let Some(bytes) = how.max_bytes {
        args.push("--max-filesize".into());
        args.push(bytes.to_string());
    }
    args
}

/// The config text that hands curl the key without a command line.
fn key_config(key: &str) -> String {
    // The key is checked to be plain characters when it is made, but this is
    // a file format: anything that could end the quoted value is dropped.
    let safe: String = key.chars().filter(|c| c.is_ascii_graphic() && *c != '"' && *c != '\\').collect();
    format!("header = \"Authorization: Bearer {safe}\"\n")
}

/// Fetches an https `url` into `to`; sends `bearer` (an access key) when there is one, and
/// reports how many bytes `to` holds about every tenth of a second, so a
/// window can draw a bar. A resumed download starts from the bytes it
/// already had.
pub fn download_with(
    url: &str,
    to: &Path,
    how: Fetch,
    bearer: Option<&Bearer>,
    progress: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let Some(bearer) = bearer else {
        return run(url, to, how, None, true, progress);
    };
    let key = bearer.key.as_str();
    let target = resolve(url, &bearer.origin, &mut |link, send| probe(link, send.then_some(key)))?;
    // The final link gets the key only when it is on the key's own host.
    let send = origin(&target).is_ok_and(|found| found == bearer.origin);
    run(&target, to, how, send.then_some(key), false, progress)
}

/// One curl run, with the progress polling.
fn run(
    url: &str,
    to: &Path,
    how: Fetch,
    key: Option<&str>,
    follow: bool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let curl = curl_path()?;
    let mut command = Command::new(curl);
    command.args(arguments(how, follow));
    if key.is_some() {
        command.args(["--config", "-"]);
    }
    let mut child = command
        .arg("--output")
        .arg(to)
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(if key.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let (Some(key), Some(mut stdin)) = (key, child.stdin.take()) {
        stdin.write_all(key_config(key).as_bytes()).map_err(|err| err.to_string())?;
        // Dropping stdin closes it, which tells curl the config is complete.
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                progress(std::fs::metadata(to).map_or(0, |meta| meta.len()));
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            Err(err) => return Err(err.to_string()),
        }
    };
    progress(std::fs::metadata(to).map_or(0, |meta| meta.len()));
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    // A partial file that was complete already: curl answers 416 ("range
    // not satisfiable"), which here means there is nothing left to fetch.
    let reason = stderr.trim().trim_start_matches("curl: ").to_string();
    if how.resume && reason.contains("416") {
        return Ok(());
    }
    Err(describe(&reason, url))
}

/// curl's complaint as a sentence a person can act on; a refused key is
/// said as such, because it means something to the user.
fn describe(reason: &str, url: &str) -> String {
    if reason.is_empty() {
        return format!("{url} couldn't be downloaded");
    }
    if reason.contains("401") || reason.contains("403") {
        return format!("the server refused the access key ({reason})");
    }
    reason.to_string()
}

/// Asks `url` what it is, without downloading it: a request for the first
/// byte only. Gives the HTTP status and, for a redirect, where it points.
fn probe(url: &str, key: Option<&str>) -> Result<(u16, String), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(curl_path()?);
    command.args([
        "--silent", "--show-error", "--proto", "=https", "--connect-timeout", "20", "--max-time", "60",
        "--retry", "2", "--retry-delay", "2", "--range", "0-0", "--max-filesize", "1024", "--header",
        "Cache-Control: no-cache", "--output", "NUL", "--write-out", "%{http_code} %{redirect_url}",
    ]);
    if key.is_some() {
        command.args(["--config", "-"]);
    }
    let mut child = command
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(if key.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    if let (Some(key), Some(mut stdin)) = (key, child.stdin.take()) {
        stdin.write_all(key_config(key).as_bytes()).map_err(|err| err.to_string())?;
    }
    let output = child.wait_with_output().map_err(|err| err.to_string())?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match parse_probe(&said) {
        Some((code, redirect)) if code != 0 => Ok((code, redirect)),
        _ => {
            let reason = String::from_utf8_lossy(&output.stderr).trim().trim_start_matches("curl: ").to_string();
            Err(if reason.is_empty() { format!("{url} couldn't be reached") } else { reason })
        }
    }
}

/// `"302 https://other/"` as a status and a link (empty when there is none).
fn parse_probe(text: &str) -> Option<(u16, String)> {
    let (code, rest) = match text.split_once(' ') {
        Some((code, rest)) => (code, rest.trim()),
        None => (text, ""),
    };
    Some((code.trim().parse().ok()?, rest.to_string()))
}

/// Follows redirects from `url` by hand, so a key is only ever sent to the
/// host it was given for, `allowed`. `ask(link, send_key)` reports a link's
/// HTTP status and where it redirects; `send_key` says whether the key may go
/// with the request. Gives the link that finally answers with success.
fn resolve(url: &str, allowed: &str, ask: &mut dyn FnMut(&str, bool) -> Answer) -> Result<String, String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_HOPS {
        let send = origin(&current)? == allowed;
        let (code, next) = ask(&current, send)?;
        match code {
            200..=299 => return Ok(current),
            301 | 302 | 303 | 307 | 308 => {
                if !bach_pack::manifest::extras::is_https(&next) {
                    return Err("the server redirected to a link that isn't https".into());
                }
                current = next;
            }
            401 | 403 => return Err(format!("the server refused the access key (HTTP {code})")),
            _ => return Err(format!("the server answered HTTP {code}")),
        }
    }
    Err("the server redirected too many times".into())
}

/// The part of an https link that decides who it is: `host:port`, lowercase.
/// A link with a user name in front of the host is refused, because what
/// follows `@` is the host and that is easy to misread.
fn origin(url: &str) -> Result<String, String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| "the link isn't https".to_string())?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return Err(format!("\"{url}\" isn't a link a key may be sent to"));
    }
    Ok(authority.to_ascii_lowercase().trim_end_matches(":443").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_refused() {
        let to = std::env::temp_dir().join(format!("bach-curl-{}", std::process::id()));
        assert!(download_with("http://example.com/", &to, Fetch::FEED, None, &mut |_| {}).is_err());
        assert!(download_with("http://example.com/", &to, Fetch::FEED, Bearer::for_feed("secret-key-1234", "https://example.com/x").as_ref(), &mut |_| {}).is_err());
        let _ = std::fs::remove_file(to);
    }

    #[test]
    fn arguments_never_allow_other_schemes() {
        for how in [Fetch::FEED, Fetch::SETUP] {
            for follow in [true, false] {
                let args = arguments(how, follow).join(" ");
                assert!(args.contains("--proto =https"), "{args}");
                assert_eq!(args.contains("--proto-redir =https"), follow, "{args}");
                assert_eq!(args.contains("--location"), follow, "{args}");
            }
        }
        assert!(arguments(Fetch::SETUP, true).join(" ").contains("--continue-at -"));
        assert!(arguments(Fetch::FEED, true).join(" ").contains("--max-filesize"));
    }

    #[test]
    fn a_key_never_reaches_a_command_line() {
        let args = arguments(Fetch::SETUP, false).join(" ");
        assert!(!args.contains("Authorization"), "{args}");
        assert_eq!(key_config("K7QF-3MWD"), "header = \"Authorization: Bearer K7QF-3MWD\"\n");
        // Whatever could end the quoted value, or add a line, is dropped.
        assert_eq!(key_config("a\"b\\c\nd e"), "header = \"Authorization: Bearer abcde\"\n");
    }

    #[test]
    fn origins_tell_hosts_apart() {
        assert_eq!(origin("https://Example.com/a/b?x=1").unwrap(), "example.com");
        assert_eq!(origin("https://example.com:443/a").unwrap(), "example.com");
        assert_eq!(origin("https://example.com:8443/a").unwrap(), "example.com:8443");
        assert_eq!(origin("https://example.com").unwrap(), "example.com");
        assert!(origin("http://example.com/").is_err());
        assert!(origin("https://").is_err());
        assert!(origin("https://good.com@evil.com/x").is_err(), "a user name in front hides the host");
        let bearer = |feed: &str| Bearer::for_feed("secret-key-1234", feed).map(|b| b.origin);
        assert_eq!(bearer("https://A.com/latest.json").as_deref(), Some("a.com"));
        assert_ne!(bearer("https://a.com/x"), bearer("https://cdn.a.com/x"));
        assert_ne!(bearer("https://a.com/x"), bearer("https://a.com:8443/x"));
        assert!(bearer("http://a.com/x").is_none() && bearer("https://u@a.com/x").is_none());
        assert_eq!(format!("{:?}", Bearer::for_feed("secret-key-1234", "https://a.com/x").unwrap()), "Bearer(.. for a.com)");
    }

    #[test]
    fn probe_answers_are_read() {
        assert_eq!(parse_probe("206 "), Some((206, String::new())));
        assert_eq!(parse_probe("302 https://cdn.example/f.exe"), Some((302, "https://cdn.example/f.exe".into())));
        assert_eq!(parse_probe("200"), Some((200, String::new())));
        assert_eq!(parse_probe("nonsense"), None);
        assert_eq!(parse_probe(""), None);
    }

    /// Plays a server: `answers` by link, and records who got the key.
    fn follow(url: &str, answers: &[(&str, u16, &str)]) -> (Result<String, String>, Vec<(String, bool)>) {
        follow_allowing(url, "a.com", answers)
    }

    fn follow_allowing(url: &str, allowed: &str, answers: &[(&str, u16, &str)]) -> (Result<String, String>, Vec<(String, bool)>) {
        let mut asked = Vec::new();
        let result = resolve(url, allowed, &mut |link, send| {
            asked.push((link.to_string(), send));
            answers
                .iter()
                .find(|(known, _, _)| *known == link)
                .map(|(_, code, next)| (*code, next.to_string()))
                .ok_or_else(|| format!("nothing at {link}"))
        });
        (result, asked)
    }

    #[test]
    fn the_key_goes_only_to_the_host_it_was_meant_for() {
        let (result, asked) = follow_allowing(
            "https://example.com/latest/setup.exe",
            "example.com",
            &[
                ("https://example.com/latest/setup.exe", 302, "https://cdn.other.net/blob/1"),
                ("https://cdn.other.net/blob/1", 206, ""),
            ],
        );
        assert_eq!(result.unwrap(), "https://cdn.other.net/blob/1");
        assert_eq!(
            asked,
            [("https://example.com/latest/setup.exe".to_string(), true), ("https://cdn.other.net/blob/1".to_string(), false)]
        );
    }

    /// A feed can name any download link, so a link on a host the app never
    /// named gets no key: not on the first request, and not on a redirect.
    #[test]
    fn a_download_link_a_feed_names_never_gets_the_key() {
        let (result, asked) = follow_allowing(
            "https://evil.example/steal.exe",
            "downloads.example.com",
            &[("https://evil.example/steal.exe", 206, "")],
        );
        assert_eq!(result.unwrap(), "https://evil.example/steal.exe");
        assert_eq!(asked, [("https://evil.example/steal.exe".to_string(), false)]);

        let (_, asked) = follow_allowing(
            "https://evil.example/a",
            "downloads.example.com",
            &[("https://evil.example/a", 302, "https://downloads.example.com/b"), ("https://downloads.example.com/b", 200, "")],
        );
        assert_eq!(
            asked.iter().map(|(_, send)| *send).collect::<Vec<_>>(),
            [false, true],
            "the key may reach its own host again, but nobody else"
        );
    }

    #[test]
    fn a_redirect_back_to_the_first_host_gets_the_key_again() {
        let (result, asked) = follow(
            "https://a.com/x",
            &[("https://a.com/x", 301, "https://b.com/y"), ("https://b.com/y", 307, "https://a.com/z"), ("https://a.com/z", 200, "")],
        );
        assert_eq!(result.unwrap(), "https://a.com/z");
        assert_eq!(asked.iter().map(|(_, send)| *send).collect::<Vec<_>>(), [true, false, true]);
    }

    #[test]
    fn redirects_to_plain_http_or_in_circles_are_refused() {
        let (result, _) = follow("https://a.com/x", &[("https://a.com/x", 302, "http://a.com/x")]);
        assert!(result.unwrap_err().contains("isn't https"));
        let (result, asked) = follow("https://a.com/x", &[("https://a.com/x", 302, "https://a.com/x")]);
        assert!(result.unwrap_err().contains("too many"));
        assert_eq!(asked.len(), MAX_HOPS + 1);
        let (result, _) = follow("https://a.com/x", &[("https://a.com/x", 302, "")]);
        assert!(result.is_err(), "a redirect that points nowhere");
    }

    #[test]
    fn a_refused_key_is_said_plainly() {
        for code in [401, 403] {
            let (result, _) = follow("https://a.com/x", &[("https://a.com/x", code, "")]);
            assert!(result.unwrap_err().contains("refused the access key"), "{code}");
        }
        let (result, _) = follow("https://a.com/x", &[("https://a.com/x", 500, "")]);
        assert!(result.unwrap_err().contains("HTTP 500"));
        assert!(describe("The requested URL returned error: 403", "u").contains("refused the access key"));
        assert!(describe("", "https://a.com/x").contains("couldn't be downloaded"));
    }
}
