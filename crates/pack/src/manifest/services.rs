//! Windows services and firewall rules: `[[services]]` and `[[firewall]]`.
//!
//! Both need administrator rights, so a config may only have them with
//! `installer.scope = "machine"` (checked in `config/services.rs`). Setup
//! only ever changes a service or a rule it can prove is its own: a service
//! whose program lies inside the install folder, a rule for a program in it.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// How a service starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStart {
    /// With Windows.
    #[default]
    Auto,
    /// With Windows, a little after the rest (it doesn't slow the sign-in).
    Delayed,
    /// When something asks for it.
    Manual,
    /// Never, until someone turns it on.
    Disabled,
}

impl ServiceStart {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "delayed" => Some(Self::Delayed),
            "manual" => Some(Self::Manual),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    /// Whether setup starts the service once it is in place.
    pub fn starts_now(self) -> bool {
        matches!(self, Self::Auto | Self::Delayed)
    }
}

/// The account a service runs as. Setup never takes a password, so these
/// are the three built-in accounts Windows needs none for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAccount {
    /// Few rights on this computer, and none on the network.
    #[default]
    LocalService,
    /// Few rights on this computer, and the computer's identity on the network.
    NetworkService,
    /// Full rights. For a service that has to manage the computer itself.
    LocalSystem,
}

impl ServiceAccount {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "local_service" => Some(Self::LocalService),
            "network_service" => Some(Self::NetworkService),
            "local_system" => Some(Self::LocalSystem),
            _ => None,
        }
    }
}

/// A Windows service the app installs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// The name Windows knows it by (`sc query NAME`).
    pub name: String,
    /// The name Services lists.
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The program, a packaged `.exe` (relative path, `/` separators).
    pub file: String,
    /// Its command line after its path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default)]
    pub start: ServiceStart,
    #[serde(default)]
    pub account: ServiceAccount,
    /// Let Windows start it again a minute after it stops by itself.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub restart_on_failure: bool,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Which way a firewall rule faces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Connections other computers open to the app.
    #[default]
    In,
    /// Connections the app opens.
    Out,
}

/// The protocol a firewall rule covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallProtocol {
    #[default]
    Any,
    Tcp,
    Udp,
}

/// The kind of network a firewall rule applies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Domain,
    Private,
    Public,
}

impl Network {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "domain" => Some(Self::Domain),
            "private" => Some(Self::Private),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

/// A Windows Firewall rule that lets the app's traffic through.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallRule {
    /// The rule's name in Windows Defender Firewall.
    pub name: String,
    /// A packaged `.exe` the rule is for. Without one, `ports` says what is let through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub protocol: FirewallProtocol,
    /// Local ports: `8080`, `8080,9000-9010`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<String>,
    /// The networks it applies on; every one when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub networks: Vec<Network>,
}

const MAX_NAME: usize = 64;

/// A service name Windows accepts and setup can safely hand to `sc.exe`.
pub fn check_service_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_NAME {
        bail!("\"{name}\" can't be a service name: use 1 to {MAX_NAME} characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) || name.starts_with('.') {
        bail!("\"{name}\" can't be a service name: use letters, digits, `.`, `-` and `_`, not starting with `.`");
    }
    Ok(())
}

fn check_text(what: &str, text: &str, limit: usize) -> Result<()> {
    if text.trim().is_empty() || text.len() > limit || text.chars().any(char::is_control) {
        bail!("{what} must be 1 to {limit} characters without control characters");
    }
    Ok(())
}

pub fn check_service(entry: &ServiceEntry) -> Result<()> {
    check_service_name(&entry.name)?;
    check_text(&format!("the display name of service {}", entry.name), &entry.display_name, 256)?;
    if let Some(description) = &entry.description {
        check_text(&format!("the description of service {}", entry.name), description, 1000)?;
    }
    if !entry.file.to_ascii_lowercase().ends_with(".exe") {
        bail!("the program of service {} is \"{}\"; it must be an .exe", entry.name, entry.file);
    }
    if entry.arguments.as_deref().is_some_and(|arguments| arguments.len() > 1000 || arguments.chars().any(char::is_control)) {
        bail!("the arguments of service {} are too long or have control characters in them", entry.name);
    }
    Ok(())
}

/// Ports as `netsh` takes them: numbers and `a-b` ranges, comma separated.
pub fn check_ports(ports: &str) -> Result<()> {
    let bad = || anyhow::anyhow!("\"{ports}\" isn't a list of ports: write them like 8080 or 8080,9000-9010");
    if ports.is_empty() || ports.len() > 200 {
        return Err(bad());
    }
    for part in ports.split(',') {
        let mut ends = part.split('-');
        let (low, high) = (ends.next().unwrap_or_default(), ends.next());
        let number = |text: &str| text.parse::<u32>().ok().filter(|port| (1..=65535).contains(port));
        let low = number(low).ok_or_else(bad)?;
        if let Some(high) = high {
            if number(high).is_none_or(|high| high < low) || ends.next().is_some() {
                return Err(bad());
            }
        }
    }
    Ok(())
}

pub fn check_firewall(rule: &FirewallRule) -> Result<()> {
    check_text("a firewall rule's name", &rule.name, 128)?;
    if rule.name.contains(['"', '\\']) || rule.name.eq_ignore_ascii_case("all") {
        bail!("\"{}\" can't be a firewall rule's name: it has a quote or a backslash in it, or is \"all\"", rule.name);
    }
    if rule.file.is_none() && rule.ports.is_none() {
        bail!("the firewall rule \"{}\" covers nothing: give it a program (file) or ports", rule.name);
    }
    if let Some(ports) = &rule.ports {
        check_ports(ports)?;
        if rule.protocol == FirewallProtocol::Any {
            bail!("the firewall rule \"{}\" names ports, so it needs protocol = \"tcp\" or \"udp\"", rule.name);
        }
    }
    if rule.file.as_deref().is_some_and(|file| !file.to_ascii_lowercase().ends_with(".exe")) {
        bail!("the program of the firewall rule \"{}\" must be an .exe", rule.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> ServiceEntry {
        ServiceEntry {
            name: name.into(),
            display_name: "Hello Agent".into(),
            description: None,
            file: "agent/agent.exe".into(),
            arguments: None,
            start: ServiceStart::Auto,
            account: ServiceAccount::LocalService,
            restart_on_failure: true,
        }
    }

    #[test]
    fn a_service_name_is_safe_for_the_command_line() {
        for good in ["HelloAgent", "hello.agent-2_x", "A"] {
            assert!(check_service_name(good).is_ok(), "{good}");
        }
        for bad in ["", ".hidden", "has space", "a/b", "a\\b", "a\"b", "a;b", "é", &"x".repeat(65)] {
            assert!(check_service_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_service_needs_an_exe_and_clean_text() {
        assert!(check_service(&service("Hello")).is_ok());
        assert!(check_service(&ServiceEntry { file: "agent/agent.dll".into(), ..service("Hello") }).is_err());
        assert!(check_service(&ServiceEntry { display_name: " ".into(), ..service("Hello") }).is_err());
        assert!(check_service(&ServiceEntry { arguments: Some("a\nb".into()), ..service("Hello") }).is_err());
        assert!(check_service(&ServiceEntry { description: Some("x".repeat(1001)), ..service("Hello") }).is_err());
    }

    #[test]
    fn ports_are_numbers_and_ranges() {
        for good in ["80", "8080,9000-9010", "1-65535", "443,80"] {
            assert!(check_ports(good).is_ok(), "{good}");
        }
        for bad in ["", "0", "65536", "80-", "-80", "90-80", "1-2-3", "a", "80,,81", "80 81", "80;81"] {
            assert!(check_ports(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_firewall_rule_covers_something_and_says_which_protocol() {
        let rule = FirewallRule {
            name: "Hello server".into(),
            file: Some("agent/agent.exe".into()),
            direction: Direction::In,
            protocol: FirewallProtocol::Any,
            ports: None,
            networks: vec![Network::Private],
        };
        assert!(check_firewall(&rule).is_ok());
        assert!(check_firewall(&FirewallRule { file: None, ..rule.clone() }).is_err(), "nothing to let through");
        let ports = FirewallRule { file: None, ports: Some("8080".into()), ..rule.clone() };
        assert!(check_firewall(&ports).is_err(), "ports need tcp or udp");
        assert!(check_firewall(&FirewallRule { protocol: FirewallProtocol::Tcp, ..ports }).is_ok());
        assert!(check_firewall(&FirewallRule { name: "a\"b".into(), ..rule.clone() }).is_err());
        assert!(check_firewall(&FirewallRule { name: "All".into(), ..rule }).is_err(), "`all` would mean every rule to netsh");
    }

    #[test]
    fn they_are_kept_small_in_the_manifest() {
        let json = serde_json::to_string(&service("Hello")).unwrap();
        assert!(!json.contains("restart_on_failure") && !json.contains("description"), "{json}");
        assert_eq!(serde_json::from_str::<ServiceEntry>(&json).unwrap(), service("Hello"));
        assert!(serde_json::from_str::<ServiceEntry>(r#"{"name":"a","display_name":"a","file":"a.exe","start":"sometimes"}"#).is_err());
    }
}
