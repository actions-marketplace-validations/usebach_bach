//! `[[services]]` and `[[firewall]]`: Windows services and Windows Firewall
//! rules, driven through `sc.exe` and `netsh.exe`, which every Windows has.
//!
//! Both need administrator rights, so only a machine-wide install has them (the
//! config refuses them otherwise) and setup is elevated by then.
//!
//! Setup only ever touches what it can prove is its own: a service is changed,
//! stopped or deleted only when its program lies inside the install folder, and
//! a service of that name that belongs to another program fails the install
//! with a sentence instead of being taken over. Arguments go to the tools one
//! by one, never through a shell, and every name was checked when the config
//! was read.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use bach_pack::manifest::{
    Direction, FirewallProtocol, FirewallRule, Manifest, Network, ServiceAccount, ServiceEntry, ServiceStart,
};
use bach_pack::paths;

use super::folders;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// How long a service gets to stop or to start.
const WAIT: Duration = Duration::from_secs(30);
/// `sc` says this when the service doesn't exist.
const NOT_FOUND: i32 = 1060;
/// `sc stop` says this when the service isn't running, `sc start` when it already is.
const NOT_RUNNING: i32 = 1062;
const ALREADY_RUNNING: i32 = 1056;

/// Why a service or a rule couldn't be set up, for the window's "{task}
/// couldn't be set up (exit code {code})".
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub task: String,
    pub code: String,
}

/// The state `sc query` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Running,
    /// Starting, stopping, paused...
    Changing,
}

// ---- What is asked of the tools: pure, so it can be tested without Windows' services ----

fn start_word(start: ServiceStart) -> &'static str {
    match start {
        ServiceStart::Auto => "auto",
        ServiceStart::Delayed => "delayed-auto",
        ServiceStart::Manual => "demand",
        ServiceStart::Disabled => "disabled",
    }
}

fn account_name(account: ServiceAccount) -> &'static str {
    match account {
        ServiceAccount::LocalService => r"NT AUTHORITY\LocalService",
        ServiceAccount::NetworkService => r"NT AUTHORITY\NetworkService",
        ServiceAccount::LocalSystem => "LocalSystem",
    }
}

/// The service's command line: its program, quoted, then its arguments.
pub fn binary_path(program: &Path, arguments: Option<&str>) -> String {
    match arguments.map(str::trim).filter(|text| !text.is_empty()) {
        Some(arguments) => format!("\"{}\" {arguments}", program.display()),
        None => format!("\"{}\"", program.display()),
    }
}

/// `sc create` (or `sc config` for a service that is there): `sc` wants the
/// `key=` and its value as two words.
pub fn define_args(verb: &str, entry: &ServiceEntry, binary: &str) -> Vec<String> {
    let mut args = vec![verb.to_string(), entry.name.clone()];
    for (key, value) in [
        ("binPath=", binary),
        ("start=", start_word(entry.start)),
        ("DisplayName=", entry.display_name.as_str()),
        ("obj=", account_name(entry.account)),
    ] {
        args.push(key.to_string());
        args.push(value.to_string());
    }
    args
}

/// Restart a minute after a crash, three times, and forget the count after a day.
pub fn failure_args(entry: &ServiceEntry) -> Vec<String> {
    let actions = if entry.restart_on_failure { "restart/60000/restart/60000/restart/60000" } else { "\"\"/0" };
    ["failure", entry.name.as_str(), "reset=", "86400", "actions=", actions].map(String::from).to_vec()
}

fn networks(rule: &FirewallRule) -> String {
    if rule.networks.is_empty() {
        return "any".into();
    }
    let word = |network: &Network| match network {
        Network::Domain => "domain",
        Network::Private => "private",
        Network::Public => "public",
    };
    rule.networks.iter().map(word).collect::<Vec<_>>().join(",")
}

fn program_of(rule: &FirewallRule, dir: &Path) -> Option<String> {
    rule.file.as_ref().map(|file| paths::under(dir, file).display().to_string())
}

fn direction_word(direction: Direction) -> &'static str {
    match direction {
        Direction::In => "in",
        Direction::Out => "out",
    }
}

/// `netsh advfirewall firewall add rule ...`.
pub fn add_rule_args(rule: &FirewallRule, dir: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["advfirewall", "firewall", "add", "rule"].map(String::from).to_vec();
    args.push(format!("name={}", rule.name));
    args.push(format!("dir={}", direction_word(rule.direction)));
    args.extend(["action=allow".to_string(), "enable=yes".to_string()]);
    if let Some(program) = program_of(rule, dir) {
        args.push(format!("program={program}"));
    }
    match rule.protocol {
        FirewallProtocol::Any => args.push("protocol=any".into()),
        FirewallProtocol::Tcp => args.push("protocol=tcp".into()),
        FirewallProtocol::Udp => args.push("protocol=udp".into()),
    }
    if let Some(ports) = &rule.ports {
        args.push(format!("localport={ports}"));
    }
    args.push(format!("profile={}", networks(rule)));
    args
}

/// `netsh advfirewall firewall delete rule ...`: by name, narrowed by program
/// (or by direction, protocol and ports) so another app's rule of the same
/// name stays.
pub fn delete_rule_args(rule: &FirewallRule, dir: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["advfirewall", "firewall", "delete", "rule"].map(String::from).to_vec();
    args.push(format!("name={}", rule.name));
    args.push(format!("dir={}", direction_word(rule.direction)));
    if let Some(program) = program_of(rule, dir) {
        args.push(format!("program={program}"));
    }
    match rule.protocol {
        FirewallProtocol::Tcp => args.push("protocol=tcp".into()),
        FirewallProtocol::Udp => args.push("protocol=udp".into()),
        FirewallProtocol::Any => {}
    }
    if let Some(ports) = &rule.ports {
        args.push(format!("localport={ports}"));
    }
    args
}

/// The state in `sc query`'s text. The labels are translated, the state names
/// (`1  STOPPED`, `4  RUNNING`) are not.
pub fn parse_state(output: &str) -> Option<State> {
    output.lines().find_map(|line| {
        let (_, rest) = line.split_once(':')?;
        let mut words = rest.split_whitespace();
        let (number, name) = (words.next()?, words.next()?);
        if !number.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        match name {
            "STOPPED" => Some(State::Stopped),
            "RUNNING" => Some(State::Running),
            "START_PENDING" | "STOP_PENDING" | "CONTINUE_PENDING" | "PAUSE_PENDING" | "PAUSED" => Some(State::Changing),
            _ => None,
        }
    })
}

/// The program in a service's `BINARY_PATH_NAME`: the quoted part, or the
/// text up to the first space.
fn program_in(binary: &str) -> &str {
    let binary = binary.trim();
    match binary.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        None => binary.split_whitespace().next().unwrap_or_default(),
    }
}

/// Whether a service's command line starts a program inside `dir`: the proof
/// that setup made it, and the only case in which setup changes it.
pub fn belongs_to(binary: &str, dir: &Path) -> bool {
    let fold = |text: &str| text.replace('/', "\\").to_lowercase();
    let program = fold(program_in(binary));
    let dir = fold(&dir.display().to_string());
    let dir = dir.trim_end_matches('\\');
    // `..` could climb out of the folder after the prefix matched.
    if program.split('\\').any(|part| part == "..") {
        return false;
    }
    !program.is_empty() && !dir.is_empty() && program.starts_with(&format!("{dir}\\"))
}

fn binary_in(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (label, rest) = line.split_once(':')?;
        label.trim().eq_ignore_ascii_case("BINARY_PATH_NAME").then(|| rest.trim().to_string())
    })
}

// ---- Running the tools ----

fn tool(name: &str, args: &[String]) -> Result<(i32, String), String> {
    let system = folders::system_dir().ok_or_else(|| "couldn't find Windows' system folder".to_string())?;
    let output = Command::new(system.join(name))
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("couldn't run {name}: {err}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    Ok((output.status.code().unwrap_or(-1), text.trim().to_string()))
}

fn sc(args: &[&str]) -> Result<(i32, String), String> {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    tool("sc.exe", &args)
}

/// The service's state, or None when there is no such service.
fn state_of(name: &str) -> Result<Option<State>, String> {
    let (code, text) = sc(&["query", name])?;
    if code == NOT_FOUND {
        return Ok(None);
    }
    if code != 0 {
        return Err(format!("sc query {name} ended with {code}: {text}"));
    }
    Ok(parse_state(&text).or(Some(State::Changing)))
}

/// The command line of an existing service.
fn binary_of(name: &str) -> Result<Option<String>, String> {
    let (code, text) = sc(&["qc", name])?;
    if code == NOT_FOUND {
        return Ok(None);
    }
    if code != 0 {
        return Err(format!("sc qc {name} ended with {code}: {text}"));
    }
    Ok(binary_in(&text))
}

fn wait_for(name: &str, wanted: State, limit: Duration) -> bool {
    let started = Instant::now();
    loop {
        if state_of(name).ok().flatten() == Some(wanted) {
            return true;
        }
        if started.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn fail(entry: &ServiceEntry, code: impl ToString, detail: &str) -> Failure {
    crate::log::line(&format!("Service {}: {detail}", entry.name));
    Failure { task: entry.display_name.clone(), code: code.to_string() }
}

/// Stops a service of ours and waits until it has stopped. False when it
/// didn't stop in time.
fn stop(entry: &ServiceEntry) -> bool {
    match sc(&["stop", &entry.name]) {
        Ok((code, _)) if code == 0 || code == NOT_RUNNING => wait_for(&entry.name, State::Stopped, WAIT),
        Ok((code, text)) => {
            crate::log::line(&format!("Couldn't stop service {} (sc ended with {code}): {text}", entry.name));
            false
        }
        Err(message) => {
            crate::log::line(&message);
            false
        }
    }
}

/// The services of `manifest` that exist and are ours.
fn ours<'a>(manifest: &'a Manifest, dir: &Path) -> Vec<&'a ServiceEntry> {
    manifest
        .system
        .services
        .iter()
        .filter(|entry| matches!(binary_of(&entry.name), Ok(Some(binary)) if belongs_to(&binary, dir)))
        .collect()
}

/// The services that were running when setup stopped them, so a failed
/// update can start them again.
#[derive(Debug, Default)]
pub struct Stopped(Vec<String>);

impl Stopped {
    fn had(&self, name: &str) -> bool {
        self.0.iter().any(|known| known.eq_ignore_ascii_case(name))
    }
}

/// Stops the running services of the earlier and the new version, so their
/// programs aren't locked while the files are swapped. A service that won't
/// stop fails the install here, with its name, rather than as a locked file later.
pub fn stop_for_update(manifest: &Manifest, previous: Option<&Manifest>, dir: &Path) -> Result<Stopped, Failure> {
    let mut stopped = Stopped::default();
    let mut seen = Vec::new();
    for source in previous.into_iter().chain(std::iter::once(manifest)) {
        for entry in ours(source, dir) {
            if seen.iter().any(|name: &String| name.eq_ignore_ascii_case(&entry.name)) {
                continue;
            }
            seen.push(entry.name.clone());
            if state_of(&entry.name).ok().flatten() != Some(State::Running) {
                continue;
            }
            crate::log::line(&format!("Stopping service {} while its files are replaced", entry.name));
            if !stop(entry) {
                return Err(fail(entry, "stop", "it didn't stop in time"));
            }
            stopped.0.push(entry.name.clone());
        }
    }
    Ok(stopped)
}

/// Puts services that setup stopped back running (an update that failed).
pub fn restart(manifest: &Manifest, stopped: &Stopped, dir: &Path) {
    for entry in ours(manifest, dir) {
        if stopped.had(&entry.name) {
            let _ = sc(&["start", &entry.name]);
        }
    }
}

/// Creates or updates the services and the firewall rules of `manifest`, and
/// takes away what `previous` had and `manifest` doesn't.
pub fn apply(manifest: &Manifest, previous: Option<&Manifest>, dir: &Path, stopped: &Stopped) -> Result<(), Failure> {
    if let Some(previous) = previous {
        for entry in ours(previous, dir) {
            if !manifest.system.services.iter().any(|now| now.name.eq_ignore_ascii_case(&entry.name)) {
                crate::log::line(&format!("Service {} isn't part of this version, so it is removed", entry.name));
                delete(entry);
            }
        }
        for rule in &previous.system.firewall {
            if !manifest.system.firewall.iter().any(|now| now.name.eq_ignore_ascii_case(&rule.name)) {
                delete_rule(rule, dir);
            }
        }
    }
    for entry in &manifest.system.services {
        define(entry, dir)?;
        let wants_running = entry.start.starts_now() || (stopped.had(&entry.name) && entry.start != ServiceStart::Disabled);
        if wants_running {
            start(entry)?;
        }
    }
    for rule in &manifest.system.firewall {
        // The rule is deleted first so an update never leaves two of it.
        delete_rule(rule, dir);
        let (code, text) = tool("netsh.exe", &add_rule_args(rule, dir)).map_err(|message| {
            crate::log::line(&message);
            Failure { task: rule.name.clone(), code: "firewall".into() }
        })?;
        if code != 0 {
            crate::log::line(&format!("Firewall rule {} couldn't be added (netsh ended with {code}): {text}", rule.name));
            return Err(Failure { task: rule.name.clone(), code: code.to_string() });
        }
    }
    Ok(())
}

fn define(entry: &ServiceEntry, dir: &Path) -> Result<(), Failure> {
    let binary = binary_path(&paths::under(dir, &entry.file), entry.arguments.as_deref());
    let verb = match binary_of(&entry.name).map_err(|message| fail(entry, "query", &message))? {
        None => "create",
        Some(existing) if belongs_to(&existing, dir) => "config",
        Some(existing) => {
            return Err(fail(entry, "exists", &format!("a service of that name already exists and starts {existing}, which isn't in this app's folder")));
        }
    };
    let (code, text) = tool("sc.exe", &define_args(verb, entry, &binary)).map_err(|message| fail(entry, "run", &message))?;
    if code != 0 {
        return Err(fail(entry, code, &format!("sc {verb} ended with {code}: {text}")));
    }
    if let Some(description) = &entry.description {
        let _ = sc(&["description", &entry.name, description]);
    }
    let (code, text) = tool("sc.exe", &failure_args(entry)).map_err(|message| fail(entry, "run", &message))?;
    if code != 0 {
        // A service without a recovery policy still works.
        crate::log::line(&format!("Couldn't set service {}'s recovery actions (sc ended with {code}): {text}", entry.name));
    }
    crate::log::line(&format!("Service {} is set up ({verb})", entry.name));
    Ok(())
}

fn start(entry: &ServiceEntry) -> Result<(), Failure> {
    let (code, text) = sc(&["start", &entry.name]).map_err(|message| fail(entry, "run", &message))?;
    if code != 0 && code != ALREADY_RUNNING {
        return Err(fail(entry, code, &format!("sc start ended with {code}: {text}")));
    }
    if !wait_for(&entry.name, State::Running, WAIT) {
        return Err(fail(entry, "start", "it didn't reach the running state"));
    }
    Ok(())
}

fn delete(entry: &ServiceEntry) {
    if state_of(&entry.name).ok().flatten() == Some(State::Running) {
        stop(entry);
    }
    match sc(&["delete", &entry.name]) {
        Ok((0, _)) => crate::log::line(&format!("Deleted service {}", entry.name)),
        Ok((code, text)) => crate::log::line(&format!("Couldn't delete service {} (sc ended with {code}): {text}", entry.name)),
        Err(message) => crate::log::line(&message),
    }
}

fn delete_rule(rule: &FirewallRule, dir: &Path) {
    // netsh ends with 1 when no rule matched, which is what a first install has.
    if let Err(message) = tool("netsh.exe", &delete_rule_args(rule, dir)) {
        crate::log::line(&message);
    }
}

/// Before removal: stops the services of this app, so their files can go.
/// A service that won't stop is logged; the file removal then reports what it holds.
pub fn stop_for_removal(manifest: &Manifest, dir: &Path) -> Stopped {
    let mut stopped = Stopped::default();
    for entry in ours(manifest, dir) {
        if state_of(&entry.name).ok().flatten() == Some(State::Running) {
            crate::log::line(&format!("Stopping service {} before it is removed", entry.name));
            if stop(entry) {
                stopped.0.push(entry.name.clone());
            }
        }
    }
    stopped
}

/// Deletes the services and the firewall rules of this app. Removing never stops for them.
pub fn remove(manifest: &Manifest, dir: &Path) {
    for entry in ours(manifest, dir) {
        delete(entry);
    }
    for rule in &manifest.system.firewall {
        delete_rule(rule, dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> ServiceEntry {
        ServiceEntry {
            name: "HelloAgent".into(),
            display_name: "Hello Agent".into(),
            description: Some("Syncs".into()),
            file: "agent/agent.exe".into(),
            arguments: Some("--service".into()),
            start: ServiceStart::Delayed,
            account: ServiceAccount::LocalService,
            restart_on_failure: true,
        }
    }

    #[test]
    fn the_command_line_is_the_quoted_program_then_its_arguments() {
        let program = Path::new(r"C:\Program Files\Hello\agent\agent.exe");
        assert_eq!(binary_path(program, Some("--service")), r#""C:\Program Files\Hello\agent\agent.exe" --service"#);
        assert_eq!(binary_path(program, None), r#""C:\Program Files\Hello\agent\agent.exe""#);
        assert_eq!(binary_path(program, Some("  ")), r#""C:\Program Files\Hello\agent\agent.exe""#);
    }

    #[test]
    fn sc_gets_each_key_and_value_as_two_words() {
        let args = define_args("create", &entry(), r#""C:\x\agent.exe" --service"#);
        assert_eq!(
            args,
            [
                "create", "HelloAgent", "binPath=", r#""C:\x\agent.exe" --service"#, "start=", "delayed-auto", "DisplayName=", "Hello Agent",
                "obj=", r"NT AUTHORITY\LocalService"
            ]
        );
        assert_eq!(define_args("config", &entry(), "x")[0], "config");
        let system = ServiceEntry { account: ServiceAccount::LocalSystem, start: ServiceStart::Manual, ..entry() };
        let args = define_args("create", &system, "x");
        assert!(args.contains(&"demand".to_string()) && args.contains(&"LocalSystem".to_string()), "{args:?}");
    }

    #[test]
    fn a_crashed_service_restarts_unless_the_app_says_not_to() {
        assert!(failure_args(&entry()).contains(&"restart/60000/restart/60000/restart/60000".to_string()));
        let quiet = ServiceEntry { restart_on_failure: false, ..entry() };
        assert!(failure_args(&quiet).contains(&"\"\"/0".to_string()));
    }

    fn rule() -> FirewallRule {
        FirewallRule {
            name: "Hello server".into(),
            file: Some("agent/agent.exe".into()),
            direction: Direction::In,
            protocol: FirewallProtocol::Tcp,
            ports: Some("8080,9000-9010".into()),
            networks: vec![Network::Private, Network::Domain],
        }
    }

    #[test]
    fn a_rule_is_added_with_everything_it_says() {
        let args = add_rule_args(&rule(), Path::new(r"C:\Program Files\Hello"));
        assert_eq!(&args[..4], ["advfirewall", "firewall", "add", "rule"]);
        for part in [
            "name=Hello server",
            "dir=in",
            "action=allow",
            "enable=yes",
            r"program=C:\Program Files\Hello\agent\agent.exe",
            "protocol=tcp",
            "localport=8080,9000-9010",
            "profile=private,domain",
        ] {
            assert!(args.contains(&part.to_string()), "{part} missing from {args:?}");
        }
        let open = FirewallRule { networks: Vec::new(), ports: None, protocol: FirewallProtocol::Any, ..rule() };
        assert!(add_rule_args(&open, Path::new(r"C:\x")).contains(&"profile=any".to_string()));
    }

    #[test]
    fn a_rule_is_deleted_by_name_narrowed_to_what_it_is() {
        let args = delete_rule_args(&rule(), Path::new(r"C:\Program Files\Hello"));
        assert_eq!(&args[..4], ["advfirewall", "firewall", "delete", "rule"]);
        assert!(args.contains(&"name=Hello server".to_string()) && args.contains(&"protocol=tcp".to_string()), "{args:?}");
        assert!(args.iter().any(|arg| arg.starts_with("program=")), "{args:?}");
        assert!(!args.contains(&"action=allow".to_string()));
    }

    #[test]
    fn the_state_is_read_from_the_numbers_not_the_labels() {
        let english = "SERVICE_NAME: Spooler\n        TYPE               : 110  WIN32_OWN_PROCESS  (interactive)\n        STATE              : 4  RUNNING\n                                (STOPPABLE, NOT_PAUSABLE)\n        WIN32_EXIT_CODE    : 0  (0x0)\n";
        assert_eq!(parse_state(english), Some(State::Running));
        let german = "SERVICE_NAME: x\n        TYP                : 10  WIN32_OWN_PROCESS\n        ZUSTAND            : 1  STOPPED\n";
        assert_eq!(parse_state(german), Some(State::Stopped));
        assert_eq!(parse_state("        STATE              : 3  STOP_PENDING\n"), Some(State::Changing));
        assert_eq!(parse_state("nothing here"), None);
    }

    #[test]
    fn a_service_is_ours_only_when_its_program_is_in_the_install_folder() {
        let dir = Path::new(r"C:\Program Files\Hello");
        assert!(belongs_to(r#""C:\Program Files\Hello\agent\agent.exe" --service"#, dir));
        assert!(belongs_to(r#""c:\program files\hello\agent.exe""#, dir), "case doesn't matter");
        assert!(!belongs_to(r#""C:\Program Files\Hello2\agent.exe""#, dir), "a folder that only starts the same");
        assert!(!belongs_to(r#""C:\Windows\System32\svchost.exe" -k netsvcs"#, dir));
        assert!(!belongs_to("", dir));
        assert!(!belongs_to(r#""C:\Program Files\Hello\..\Other\x.exe""#, dir), "`..` climbs out of the folder");
    }

    /// Read-only, and on every Windows: the event log service always runs, and a
    /// made-up name never exists. This is `sc.exe`'s real output, in this Windows' language.
    #[test]
    fn sc_is_read_the_way_this_windows_prints_it() {
        assert_eq!(state_of("EventLog"), Ok(Some(State::Running)));
        assert_eq!(state_of("BachNoSuchService9"), Ok(None));
        let binary = binary_of("EventLog").unwrap().expect("EventLog has a program");
        assert!(binary.to_lowercase().contains("svchost.exe"), "{binary}");
        assert!(!belongs_to(&binary, Path::new(r"C:\Program Files\Hello")));
        assert_eq!(binary_of("BachNoSuchService9"), Ok(None));
    }

    #[test]
    fn the_program_is_read_from_the_service_listing() {
        let listing = "SERVICE_NAME: x\n        START_TYPE         : 2   AUTO_START\n        BINARY_PATH_NAME   : \"C:\\Program Files\\Hello\\a.exe\" --service\n        DISPLAY_NAME       : Hello\n";
        assert_eq!(binary_in(listing).as_deref(), Some(r#""C:\Program Files\Hello\a.exe" --service"#));
        assert_eq!(binary_in("DISPLAY_NAME : x"), None);
    }
}
