//! `[[registry]]`, `[[env]]` and `[[context_menu]]`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::manifest::system::{self, ContextMenuEntry, ContextTarget, EnvVar, Migrate, RegistryData, RegistryValue, RegistryView};
use crate::manifest::SystemChanges;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySection {
    key: String,
    name: Option<String>,
    /// `"string"` (default), `"expand"` or `"dword"`.
    #[serde(rename = "type")]
    kind: Option<String>,
    value: toml::Value,
    /// `"64"` (default), `"32"` or `"both"`: the registry view.
    view: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvSection {
    name: String,
    value: String,
    /// Lets the variable have a name that doesn't start with the app's id.
    #[serde(default)]
    allow: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMenuSection {
    name: String,
    /// One place or a list: `"files"`, `"folders"`, `"background"`, `".ext"`.
    on: Option<toml::Value>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateSection {
    uninstall_key: String,
    arguments: Option<String>,
    #[serde(default)]
    elevated: bool,
}

/// Who the app is, so keys and variables can be checked against it.
pub struct Owner<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub publisher: &'a str,
}

/// Whether any `[[registry]]` value leaves its view unsaid.
pub fn registry_leaves_view_open(registry: &[RegistrySection]) -> bool {
    registry.iter().any(|section| section.view.is_none())
}

/// The `Machine` field of a program's PE header: 0x14C for a 32-bit x86 program.
pub fn pe_machine(path: &std::path::Path) -> Option<u16> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(4096).read_to_end(&mut head).ok()?;
    let pe = u32::from_le_bytes(head.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if head.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let machine = head.get(pe + 4..pe + 6)?;
    Some(u16::from_le_bytes([machine[0], machine[1]]))
}

pub fn changes(
    registry: &[RegistrySection],
    env: &[EnvSection],
    menu: &[ContextMenuSection],
    migrate: &[MigrateSection],
    owner: &Owner,
    warnings: &mut Vec<String>,
) -> Result<SystemChanges> {
    let mut changes = SystemChanges::default();
    for (index, section) in migrate.iter().enumerate() {
        let entry = Migrate {
            uninstall_key: section.uninstall_key.trim().to_string(),
            arguments: super::optional(&section.arguments),
            elevated: section.elevated,
        };
        system::check_migrate(&entry).map_err(|err| anyhow::anyhow!("migrate[{index}]: {err}"))?;
        if changes.migrate.iter().any(|known| known.uninstall_key.eq_ignore_ascii_case(&entry.uninstall_key)) {
            bail!("migrate[{index}].uninstall_key is \"{}\", which is listed twice", entry.uninstall_key);
        }
        changes.migrate.push(entry);
    }
    for (index, section) in registry.iter().enumerate() {
        let at = format!("registry[{index}]");
        let kind = section.kind.as_deref().map(|kind| kind.trim().to_ascii_lowercase());
        let data = match (kind.as_deref(), &section.value) {
            (None | Some("string"), toml::Value::String(text)) => RegistryData::String(text.clone()),
            (Some("expand"), toml::Value::String(text)) => RegistryData::Expand(text.clone()),
            (Some("dword"), toml::Value::Integer(number)) => match u32::try_from(*number) {
                Ok(number) => RegistryData::Dword(number),
                Err(_) => bail!("{at}.value is {number}; a dword is a whole number from 0 to 4294967295"),
            },
            (Some("string" | "expand"), _) | (None, _) => bail!("{at}.value must be text (a string or expand value)"),
            (Some("dword"), _) => bail!("{at}.value must be a whole number for type = \"dword\""),
            (Some(other), _) => bail!("{at}.type is \"{other}\"; use \"string\", \"expand\" or \"dword\""),
        };
        let view = match &section.view {
            None => RegistryView::default(),
            Some(text) => {
                RegistryView::parse(text).with_context(|| format!("{at}.view is \"{text}\"; use \"64\" (default), \"32\" or \"both\""))?
            }
        };
        let value = RegistryValue {
            key: section.key.trim().trim_end_matches('\\').to_string(),
            name: section.name.clone().unwrap_or_default(),
            data,
            view,
        };
        system::check_registry_value(&value).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        if !system::key_is_named_after(&value.key, &[owner.publisher, owner.name, owner.id]) {
            warnings.push(format!(
                "{at}.key is \"{}\", which isn't below a folder named after app.publisher, app.name or app.id; check it for a typo",
                value.key
            ));
        }
        if changes.registry.iter().any(|known| known.key.eq_ignore_ascii_case(&value.key) && known.name.eq_ignore_ascii_case(&value.name)) {
            bail!("{at} sets {}\\{} a second time", value.key, value.name);
        }
        changes.registry.push(value);
    }
    for (index, section) in env.iter().enumerate() {
        let at = format!("env[{index}]");
        let var = EnvVar { name: section.name.trim().to_string(), value: section.value.clone() };
        system::check_env_var(&var).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        let prefix = system::env_prefix(owner.id);
        if !var.name.to_ascii_uppercase().starts_with(&prefix) {
            if !section.allow {
                bail!(
                    "{at}.name is \"{}\", which doesn't start with {prefix}; use a name of your own such as {prefix}HOME, \
                     or set allow = true to change a variable other programs may read",
                    var.name
                );
            }
            warnings.push(format!("{at}.name is \"{}\", a variable other programs may read; it is allowed by allow = true", var.name));
        }
        if changes.environment.iter().any(|known| known.name.eq_ignore_ascii_case(&var.name)) {
            bail!("{at}.name is \"{}\", which is set twice", var.name);
        }
        changes.environment.push(var);
    }
    for (index, section) in menu.iter().enumerate() {
        let at = format!("context_menu[{index}]");
        let words: Vec<&str> = match &section.on {
            Some(toml::Value::String(word)) => vec![word.as_str()],
            Some(toml::Value::Array(items)) => items.iter().filter_map(toml::Value::as_str).collect(),
            _ => Vec::new(),
        };
        if words.is_empty() {
            bail!("{at}.on is missing; use \"files\", \"folders\", \"background\", a file extension like \".txt\", or a list of them");
        }
        let mut on = Vec::new();
        for word in words {
            let target = ContextTarget::parse(word).map_err(|err| anyhow::anyhow!("{at}.on: {err}"))?;
            if !on.contains(&target) {
                on.push(target);
            }
        }
        let entry = ContextMenuEntry { name: section.name.trim().to_string(), on, arguments: super::optional(&section.arguments) };
        system::check_context_entry(&entry).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        changes.context_menu.push(entry);
    }
    system::check(&changes)?;
    Ok(changes)
}
