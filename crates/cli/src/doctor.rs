//! `bach doctor`: whether everything a build needs is in place, in a few
//! lines a person or an agent can act on.

use std::path::Path;

use anyhow::Result;

use crate::output::field;
use crate::runtime::{self, Found};

pub fn doctor(config: &Path) -> Result<()> {
    say!("Bach {} on {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    let mut problems = 0;
    let mut signing_program = None;
    match runtime::locate(None) {
        Ok(Found::Here(path)) => match runtime::read(&path) {
            Ok(_) => {
                let hash = runtime::hash_of(&path).unwrap_or_default();
                field("Runtime", format!("{} (SHA-256 {hash})", path.display()));
                field("Runtime trust", runtime::trust_note());
            }
            Err(err) => {
                problems += 1;
                field("Runtime", format!("{} is unusable: {err:#}", path.display()));
            }
        },
        Ok(Found::Downloadable(url)) => {
            field("Runtime", format!("not here yet; the first build downloads it from {url}"));
            field("Runtime trust", runtime::trust_note());
        }
        Err(err) => {
            problems += 1;
            field("Runtime", format!("{err:#}"));
        }
    }

    if config.is_file() {
        match bach_pack::config::load(config) {
            Ok(project) => {
                let manifest = &project.manifest;
                signing_program = project.signing.as_ref().map(|signing| signing.program.clone());
                let warnings = match project.warnings.len() {
                    0 => String::new(),
                    1 => " (1 warning: run bach check)".into(),
                    count => format!(" ({count} warnings: run bach check)"),
                };
                field("Config", format!("{} is valid: {} {}{warnings}", config.display(), manifest.app.name, manifest.app.version));
            }
            Err(err) => {
                problems += 1;
                field("Config", format!("{err:#}"));
            }
        }
    } else {
        field("Config", format!("no {} here; `bach init` writes one", config.display()));
    }

    field("Preview", if cfg!(windows) { "available" } else { "needs Windows (bach check works here)" });
    match signing_program {
        Some(program) => field("Signing", format!("[signing] runs {program} when you build with --sign (or BACH_SIGN=1)")),
        None => field(
            "Signing",
            match crate::sign::known_signer() {
                Some(tool) => format!("off, {tool} found (add [signing] and build with --sign)"),
                None => "off (optional: [signing] in bach.toml, see docs/signing.md)".to_string(),
            },
        ),
    }
    say!();
    crate::output::result("ready", problems == 0);
    crate::output::result("problems", problems);
    if problems == 0 {
        say!("Ready to build.");
    } else {
        say!("Fix the {} above, then run bach doctor again.", if problems == 1 { "problem" } else { "problems" });
    }
    Ok(())
}
