//! `[[services]]` and `[[firewall]]`: Windows services and Windows Firewall
//! rules for the app. Both need administrator rights, so both need
//! `installer.scope = "machine"`.

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::manifest::services::{
    check_firewall, check_service, Direction, FirewallProtocol, FirewallRule, Network, ServiceAccount, ServiceEntry, ServiceStart,
};
use crate::manifest::{FileEntry, SystemChanges};
use crate::paths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceSection {
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    /// A packaged `.exe`; the app's own program when left out.
    file: Option<String>,
    arguments: Option<String>,
    /// `"auto"` (default), `"delayed"`, `"manual"` or `"disabled"`.
    start: Option<String>,
    /// `"local_service"` (default), `"network_service"` or `"local_system"`.
    account: Option<String>,
    restart_on_failure: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirewallSection {
    name: String,
    file: Option<String>,
    /// `"in"` (default) or `"out"`.
    direction: Option<String>,
    /// `"any"` (default), `"tcp"` or `"udp"`.
    protocol: Option<String>,
    /// `8080`, `"8080"` or `"8080,9000-9010"`.
    ports: Option<toml::Value>,
    /// One network or a list: `"domain"`, `"private"`, `"public"`. Every one when left out.
    networks: Option<toml::Value>,
}

/// The packaged path of a program, which has to be one of the installed files.
fn program(at: &str, setting: &str, text: &str, files: &[FileEntry]) -> Result<String> {
    let file = paths::normalize(text.trim()).map_err(|err| anyhow::anyhow!("{at}.{setting}: {err}"))?;
    if !file.to_ascii_lowercase().ends_with(".exe") {
        bail!("{at}.{setting} is \"{file}\"; it must be an .exe");
    }
    match files.iter().find(|entry| entry.path.eq_ignore_ascii_case(&file)) {
        Some(entry) => Ok(entry.path.clone()),
        None => bail!("{at}.{setting} is \"{file}\", but no installed file has that path (is it excluded?)"),
    }
}

fn words(value: &Option<toml::Value>) -> Vec<String> {
    match value {
        Some(toml::Value::String(word)) => vec![word.clone()],
        Some(toml::Value::Array(items)) => items.iter().filter_map(toml::Value::as_str).map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Adds the services and firewall rules to `changes`.
pub fn add(
    changes: &mut SystemChanges,
    services: &[ServiceSection],
    firewall: &[FirewallSection],
    files: &[FileEntry],
    app_executable: &str,
    machine: bool,
) -> Result<()> {
    if (!services.is_empty() || !firewall.is_empty()) && !machine {
        let which = if services.is_empty() { "[[firewall]]" } else { "[[services]]" };
        bail!(
            "{which} needs administrator rights, so it needs installer.scope = \"machine\" \
             (a per-user install never asks for them)"
        );
    }
    for (index, section) in services.iter().enumerate() {
        let at = format!("services[{index}]");
        let file = program(&at, "file", section.file.as_deref().unwrap_or(app_executable), files)?;
        let start = match section.start.as_deref() {
            None => ServiceStart::Auto,
            Some(word) => ServiceStart::parse(word)
                .ok_or_else(|| anyhow::anyhow!("{at}.start is \"{word}\"; use \"auto\", \"delayed\", \"manual\" or \"disabled\""))?,
        };
        let account = match section.account.as_deref() {
            None => ServiceAccount::LocalService,
            Some(word) => ServiceAccount::parse(word).ok_or_else(|| {
                anyhow::anyhow!("{at}.account is \"{word}\"; use \"local_service\", \"network_service\" or \"local_system\"")
            })?,
        };
        let name = section.name.trim().to_string();
        let entry = ServiceEntry {
            display_name: super::optional(&section.display_name).unwrap_or_else(|| name.clone()),
            description: super::optional(&section.description),
            name,
            file,
            arguments: super::optional(&section.arguments),
            start,
            account,
            restart_on_failure: section.restart_on_failure.unwrap_or(true),
        };
        check_service(&entry).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        changes.services.push(entry);
    }
    for (index, section) in firewall.iter().enumerate() {
        let at = format!("firewall[{index}]");
        let file = section.file.as_deref().map(|file| program(&at, "file", file, files)).transpose()?;
        let direction = match section.direction.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("in") => Direction::In,
            Some("out") => Direction::Out,
            Some(word) => bail!("{at}.direction is \"{word}\"; use \"in\" or \"out\""),
        };
        let protocol = match section.protocol.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("any") => FirewallProtocol::Any,
            Some("tcp") => FirewallProtocol::Tcp,
            Some("udp") => FirewallProtocol::Udp,
            Some(word) => bail!("{at}.protocol is \"{word}\"; use \"any\", \"tcp\" or \"udp\""),
        };
        let ports = match &section.ports {
            None => None,
            Some(toml::Value::Integer(port)) => Some(port.to_string()),
            Some(toml::Value::String(text)) => Some(text.trim().to_string()),
            Some(_) => bail!("{at}.ports is a port number or a text like \"8080,9000-9010\""),
        };
        let mut networks = Vec::new();
        for word in words(&section.networks) {
            let network = Network::parse(&word)
                .ok_or_else(|| anyhow::anyhow!("{at}.networks has \"{word}\"; use \"domain\", \"private\" or \"public\""))?;
            if !networks.contains(&network) {
                networks.push(network);
            }
        }
        let rule = FirewallRule { name: section.name.trim().to_string(), file, direction, protocol, ports, networks };
        check_firewall(&rule).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        changes.firewall.push(rule);
    }
    crate::manifest::system::check(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<FileEntry> {
        vec![FileEntry::new("agent.exe", 10), FileEntry::new("bin/tool.exe", 10), FileEntry::new("readme.txt", 1)]
    }

    fn sections(toml: &str) -> (Vec<ServiceSection>, Vec<FirewallSection>) {
        #[derive(Deserialize)]
        struct Both {
            #[serde(default)]
            services: Vec<ServiceSection>,
            #[serde(default)]
            firewall: Vec<FirewallSection>,
        }
        let both: Both = toml::from_str(toml).unwrap();
        (both.services, both.firewall)
    }

    fn run(toml: &str, machine: bool) -> Result<SystemChanges> {
        let (services, firewall) = sections(toml);
        let mut changes = SystemChanges::default();
        add(&mut changes, &services, &firewall, &files(), "agent.exe", machine).map(|()| changes)
    }

    #[test]
    fn a_service_takes_its_defaults() {
        let changes = run("[[services]]\nname = \"HelloAgent\"\n", true).unwrap();
        let service = &changes.services[0];
        assert_eq!((service.name.as_str(), service.display_name.as_str(), service.file.as_str()), ("HelloAgent", "HelloAgent", "agent.exe"));
        assert_eq!((service.start, service.account, service.restart_on_failure), (ServiceStart::Auto, ServiceAccount::LocalService, true));
    }

    #[test]
    fn everything_a_service_can_set() {
        let toml = "[[services]]\nname = \"HelloAgent\"\ndisplay_name = \"Hello Agent\"\ndescription = \"Syncs\"\nfile = \"bin/tool.exe\"\n\
                    arguments = \"--service\"\nstart = \"delayed\"\naccount = \"network_service\"\nrestart_on_failure = false\n";
        let service = &run(toml, true).unwrap().services[0];
        assert_eq!((service.file.as_str(), service.arguments.as_deref(), service.description.as_deref()), ("bin/tool.exe", Some("--service"), Some("Syncs")));
        assert_eq!((service.start, service.account, service.restart_on_failure), (ServiceStart::Delayed, ServiceAccount::NetworkService, false));
    }

    #[test]
    fn they_need_a_machine_wide_install() {
        let err = run("[[services]]\nname = \"A\"\n", false).unwrap_err().to_string();
        assert!(err.contains("[[services]]") && err.contains("installer.scope = \"machine\""), "{err}");
        let err = run("[[firewall]]\nname = \"A\"\nfile = \"agent.exe\"\n", false).unwrap_err().to_string();
        assert!(err.contains("[[firewall]]"), "{err}");
        assert!(run("", false).is_ok(), "an app with neither needs nothing");
    }

    #[test]
    fn each_mistake_names_its_setting() {
        for (toml, part) in [
            ("[[services]]\nname = \"A\"\nfile = \"missing.exe\"\n", "services[0].file"),
            ("[[services]]\nname = \"A\"\nfile = \"readme.txt\"\n", "must be an .exe"),
            ("[[services]]\nname = \"A\"\nstart = \"often\"\n", "services[0].start"),
            ("[[services]]\nname = \"A\"\naccount = \"admin\"\n", "services[0].account"),
            ("[[services]]\nname = \"a b\"\n", "service name"),
            ("[[services]]\nname = \"A\"\n[[services]]\nname = \"a\"\n", "used twice"),
            ("[[firewall]]\nname = \"A\"\n", "covers nothing"),
            ("[[firewall]]\nname = \"A\"\nports = 80\n", "tcp"),
            ("[[firewall]]\nname = \"A\"\nfile = \"agent.exe\"\ndirection = \"up\"\n", "firewall[0].direction"),
            ("[[firewall]]\nname = \"A\"\nfile = \"agent.exe\"\nnetworks = \"home\"\n", "firewall[0].networks"),
        ] {
            let err = run(toml, true).unwrap_err().to_string();
            assert!(err.contains(part), "{toml} -> {err}");
        }
    }

    #[test]
    fn a_firewall_rule_for_ports_and_for_a_program() {
        let toml = "[[firewall]]\nname = \"Hello server\"\nports = \"8080,9000-9010\"\nprotocol = \"TCP\"\nnetworks = [\"private\", \"domain\", \"private\"]\n\
                    [[firewall]]\nname = \"Hello agent\"\nfile = \"agent.exe\"\ndirection = \"out\"\n";
        let changes = run(toml, true).unwrap();
        assert_eq!(changes.firewall[0].ports.as_deref(), Some("8080,9000-9010"));
        assert_eq!(changes.firewall[0].networks, [Network::Private, Network::Domain], "a repeated network counts once");
        assert_eq!((changes.firewall[1].file.as_deref(), changes.firewall[1].direction), (Some("agent.exe"), Direction::Out));
    }
}
