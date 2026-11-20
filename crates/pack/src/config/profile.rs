//! `[profiles.NAME]`: other builds of the same app from the same `bach.toml`.
//!
//! A profile is a set of changes laid over the rest of the file: the settings
//! it names replace the ones above, tables are merged key by key, and an array
//! (like `[[components]]`) is replaced as a whole. So a profile for a company
//! writes only what differs:
//!
//! ```toml
//! [profiles.enterprise.installer]
//! scope = "machine"
//! desktop_shortcut = false
//!
//! [profiles.portable]
//! kind = "zip"
//! ```
//!
//! `kind = "zip"` makes a ZIP of the files instead of a setup file. The
//! profile's own keys are `kind`, `description` and `output`; everything else
//! is a section of `bach.toml`. Profiles never nest, and a profile can't change
//! `config_version`.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use toml::{Table, Value};

/// What a profile builds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A setup file, like the main build.
    #[default]
    Setup,
    /// The files in a ZIP, to unpack and run without installing.
    Zip,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Setup => "setup",
            Kind::Zip => "zip",
        }
    }
}

/// One profile of a config, for listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub kind: Kind,
    pub description: Option<String>,
}

/// What laying a profile over the config gave.
pub(super) struct Applied {
    pub table: Table,
    pub kind: Kind,
    /// The profile's own `output`, when it names one.
    pub output: Option<String>,
}

/// The sections of `bach.toml` a profile may change.
const SECTIONS: &[&str] = &[
    "app",
    "files",
    "installer",
    "uninstall",
    "components",
    "protocols",
    "file_types",
    "text",
    "requirements",
    "shortcuts",
    "run",
    "registry",
    "env",
    "context_menu",
    "migrate",
    "updates",
    "access",
    "signing",
    "style",
    "layout",
];

/// A profile's name ends up in file names and on the command line.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn parse_kind(name: &str, table: &Table) -> Result<Kind> {
    match table.get("kind") {
        None => Ok(Kind::Setup),
        Some(Value::String(kind)) => match kind.trim().to_ascii_lowercase().as_str() {
            "setup" => Ok(Kind::Setup),
            "zip" => Ok(Kind::Zip),
            _ => bail!("profiles.{name}.kind is \"{kind}\"; use \"setup\" (a setup file) or \"zip\" (the files in a ZIP)"),
        },
        Some(_) => bail!("profiles.{name}.kind must be text: \"setup\" or \"zip\""),
    }
}

/// Checks every profile's shape: its name, its `kind`, and that it only names
/// settings `bach.toml` has. Values are checked when the profile is built.
pub(super) fn check(profiles: &BTreeMap<String, Value>) -> Result<()> {
    for (name, value) in profiles {
        if !valid_name(name) {
            bail!("the profile \"{name}\" can't be used: a profile's name is lowercase letters, digits and '-', up to 32 characters");
        }
        let Value::Table(table) = value else {
            bail!("profiles.{name} must be a table; write [profiles.{name}] and the settings that differ under it");
        };
        parse_kind(name, table)?;
        for (key, value) in table {
            match key.as_str() {
                "kind" => {}
                "description" | "output" if !value.is_str() => bail!("profiles.{name}.{key} must be text"),
                "description" | "output" => {}
                key if SECTIONS.contains(&key) => {}
                "profiles" => bail!("profiles.{name} has profiles inside it; profiles don't nest"),
                "config_version" => bail!("profiles.{name} sets config_version; a profile can't change it"),
                other => bail!(
                    "profiles.{name}.{other} isn't a setting a profile knows. Use kind, description, output, or a section of bach.toml (app, files, installer, ...)"
                ),
            }
        }
        if let Some(Value::String(output)) = table.get("output") {
            if output.trim().is_empty() {
                bail!("profiles.{name}.output is empty");
            }
        }
    }
    Ok(())
}

/// The profiles of the config text, in name order.
pub fn list(text: &str) -> Result<Vec<Profile>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let table: Table = toml::from_str(text).map_err(|err| anyhow::anyhow!("{}", err.to_string().trim()))?;
    let Some(profiles) = table.get("profiles") else {
        return Ok(Vec::new());
    };
    let Value::Table(profiles) = profiles else {
        bail!("profiles must be a table of profiles: write [profiles.NAME]");
    };
    let profiles: BTreeMap<String, Value> = profiles.clone().into_iter().collect();
    check(&profiles)?;
    profiles
        .iter()
        .map(|(name, value)| {
            let table = value.as_table().cloned().unwrap_or_default();
            Ok(Profile {
                name: name.clone(),
                kind: parse_kind(name, &table)?,
                description: table.get("description").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

/// Lays profile `name` over the config in `text`.
pub(super) fn apply(text: &str, name: &str) -> Result<Applied> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut table: Table = toml::from_str(text).map_err(|err| anyhow::anyhow!("{}", err.to_string().trim()))?;
    let known = list(text)?;
    let Some(profile) = table.remove("profiles").and_then(|profiles| match profiles {
        Value::Table(mut profiles) => profiles.remove(name),
        _ => None,
    }) else {
        let names: Vec<&str> = known.iter().map(|profile| profile.name.as_str()).collect();
        if names.is_empty() {
            bail!("there is no profile \"{name}\": bach.toml has no [profiles.{name}] (or any other profile)");
        }
        bail!("there is no profile \"{name}\" in bach.toml; its profiles are {}", names.join(", "));
    };
    let Value::Table(mut profile) = profile else {
        bail!("profiles.{name} must be a table");
    };
    let kind = parse_kind(name, &profile)?;
    let output = profile.remove("output").and_then(|value| value.as_str().map(str::to_string));
    profile.remove("kind");
    profile.remove("description");
    merge(&mut table, profile);
    Ok(Applied { table, kind, output })
}

/// Lays `over` on `base`: tables merge key by key, anything else is replaced.
fn merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(below)), Value::Table(above)) => merge(below, above),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[app]
name = "Hello"
version = "1.0.0"
publisher = "Example"
executable = "hello.exe"

[files]
source = "app"

[installer]
desktop_shortcut = true

[[components]]
id = "samples"
title = "Samples"
files = ["samples/**"]

[profiles.enterprise]
description = "For IT departments"

[profiles.enterprise.installer]
scope = "machine"
desktop_shortcut = false

[profiles.portable]
kind = "zip"
output = "dist/Hello-Portable.zip"

[[profiles.portable.components]]
id = "samples"
title = "Samples"
files = ["samples/**"]
default = true
"#;

    #[test]
    fn profiles_are_listed_with_their_kind() {
        let profiles = list(BASE).unwrap();
        let names: Vec<(&str, Kind)> = profiles.iter().map(|profile| (profile.name.as_str(), profile.kind)).collect();
        assert_eq!(names, [("enterprise", Kind::Setup), ("portable", Kind::Zip)]);
        assert_eq!(profiles[0].description.as_deref(), Some("For IT departments"));
        assert!(list("[app]\nname = \"x\"\n").unwrap().is_empty());
    }

    #[test]
    fn a_profile_changes_only_what_it_names() {
        let applied = apply(BASE, "enterprise").unwrap();
        let installer = applied.table["installer"].as_table().unwrap();
        assert_eq!(installer["scope"].as_str(), Some("machine"));
        assert_eq!(installer["desktop_shortcut"].as_bool(), Some(false), "the profile's value wins");
        assert_eq!(applied.table["app"]["name"].as_str(), Some("Hello"), "everything else stays");
        assert!(applied.table.get("profiles").is_none(), "the file the build reads has no profiles left");
        assert_eq!(applied.kind, Kind::Setup);

        let portable = apply(BASE, "portable").unwrap();
        assert_eq!((portable.kind, portable.output.as_deref()), (Kind::Zip, Some("dist/Hello-Portable.zip")));
        let components = portable.table["components"].as_array().unwrap();
        assert_eq!(components.len(), 1, "an array is replaced, not added to");
        assert_eq!(components[0]["default"].as_bool(), Some(true));
        assert!(portable.table["installer"].get("scope").is_none(), "profiles don't leak into each other");
    }

    #[test]
    fn a_missing_profile_names_the_ones_there_are() {
        let error = apply(BASE, "nope").err().unwrap().to_string();
        assert!(error.contains("enterprise, portable"), "{error}");
        let error = apply("[app]\nname = \"x\"\n", "nope").err().unwrap().to_string();
        assert!(error.contains("no [profiles.nope]"), "{error}");
    }

    #[test]
    fn mistakes_in_a_profile_are_one_sentence_naming_it() {
        let broken = |extra: &str| list(&format!("[profiles.p]\n{extra}")).err().map(|error| error.to_string()).unwrap_or_default();
        assert!(broken("kind = \"msi\"").contains("profiles.p.kind is \"msi\""));
        assert!(broken("kind = 3").contains("must be text"));
        assert!(broken("colour = \"red\"").contains("profiles.p.colour isn't a setting"));
        assert!(broken("config_version = 1").contains("can't change it"));
        assert!(broken("output = \"\"").contains("is empty"));
        assert!(broken("[profiles.p.profiles.q]").contains("don't nest"));
        assert!(list("[profiles]\nx = 1\n").err().unwrap().to_string().contains("profiles.x must be a table"));
        assert!(list("[profiles.\"Big Name\"]\n").err().unwrap().to_string().contains("lowercase"));
    }

    #[test]
    fn names_are_safe_in_a_file_name() {
        for good in ["enterprise", "portable", "x64", "a-b"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "Big", "a b", "a/b", "-a", "a_b", "../x", &"a".repeat(33)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
