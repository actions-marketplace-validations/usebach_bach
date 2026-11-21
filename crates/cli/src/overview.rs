//! What plain `bach` (and `bach --help`) prints: every command, grouped by
//! what you are doing, instead of clap's one long alphabet-free list.
//!
//! Colour comes from `anstream`: it is off when the output isn't a terminal or
//! `NO_COLOR` is set, and on Windows it switches the console to understand it.

use anstyle::{AnsiColor, Effects, Style};

const HEADING: Style = Style::new().effects(Effects::BOLD);
const COMMAND: Style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
const DIM: Style = Style::new().effects(Effects::DIMMED);

/// The groups, in the order a project meets them. Every public command is in
/// exactly one (a test keeps it so).
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Start",
        &[
            ("init", "Write a starter bach.toml"),
            ("import", "Turn an Inno Setup or NSIS script into a bach.toml"),
            ("check", "Read bach.toml and list what would be installed"),
            ("build", "Build the setup file"),
        ],
    ),
    (
        "Try it",
        &[
            ("preview", "Open the installer window without installing anything"),
            ("watch", "Keep the preview open and reload it when bach.toml changes"),
            ("test", "Install, start and remove the setup file on a clean Windows"),
        ],
    ),
    (
        "Look and layout",
        &[
            ("styles", "List the built-in styles of the window"),
            ("layouts", "List the built-in layouts of the window"),
            ("profiles", "List the other builds of this bach.toml"),
        ],
    ),
    (
        "Inspect a setup file",
        &[
            ("inspect", "Show what it installs"),
            ("verify", "Check it for damage without installing it"),
            ("diff", "Show what changed between two versions"),
        ],
    ),
    (
        "Release",
        &[
            ("version", "Show or raise the version in every file that has one"),
            ("workflow", "Add the one-click GitHub release workflow"),
            ("sign", "Sign a setup file"),
            ("feed-key", "Make the key pair that signs update feeds"),
            ("feed", "Write the update feed (latest.json)"),
            ("key", "Make access keys for a protected app"),
        ],
    ),
    (
        "Publish and deploy",
        &[
            ("winget", "Write the winget manifests"),
            ("choco", "Write the Chocolatey package"),
            ("page", "Write a download page"),
            ("intune", "Write the Intune settings and detection script"),
        ],
    ),
    ("Bach itself", &[("doctor", "Check that everything a build needs is in place"), ("update", "Update bach to the newest release")]),
];

/// Prints the overview.
pub fn print() {
    use anstream::println;

    let version = env!("CARGO_PKG_VERSION");
    println!("{HEADING}Bach {version}{HEADING:#}  {DIM}modern, clean Windows installers from one bach.toml{DIM:#}");
    let width = GROUPS.iter().flat_map(|(_, commands)| commands.iter()).map(|(name, _)| name.len()).max().unwrap_or(0);
    for (group, commands) in GROUPS {
        println!();
        println!("{HEADING}{group}{HEADING:#}");
        for (name, what) in *commands {
            println!("  {COMMAND}{name:<width$}{COMMAND:#}  {what}");
        }
    }
    println!();
    println!("{DIM}bach <command> --help{DIM:#}   the options of one command");
    println!("{DIM}bach --json ...{DIM:#}         the result as one JSON object, for scripts");
    println!("{DIM}https://usebach.app/docs{DIM:#}");
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn only_a_bare_bach_or_a_plain_help_asks_for_the_overview() {
        let args = |list: &[&str]| list.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        for asked in [&[][..], &["help"], &["--help"], &["-h"]] {
            assert!(crate::wants_overview(&args(asked)), "{asked:?}");
        }
        for other in [&["build"][..], &["build", "--help"], &["--json"], &["help", "build"], &["--version"]] {
            assert!(!crate::wants_overview(&args(other)), "{other:?}");
        }
    }

    #[test]
    fn every_command_is_listed_once() {
        let listed: Vec<&str> = GROUPS.iter().flat_map(|(_, commands)| commands.iter().map(|(name, _)| *name)).collect();
        for command in crate::Cli::command().get_subcommands().filter(|command| !command.is_hide_set()) {
            let count = listed.iter().filter(|name| **name == command.get_name()).count();
            assert_eq!(count, 1, "`bach {}` must be in the overview exactly once", command.get_name());
        }
        for name in &listed {
            assert!(crate::Cli::command().find_subcommand(name).is_some(), "`{name}` in the overview isn't a command");
        }
    }
}
