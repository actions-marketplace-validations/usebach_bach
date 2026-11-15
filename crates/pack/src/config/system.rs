//! `[[registry]]`, `[[env]]` and `[[context_menu]]`.

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::manifest::system::{self, ContextMenuEntry, ContextTarget, EnvVar, RegistryData, RegistryValue};
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvSection {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMenuSection {
    name: String,
    /// One place or a list: `"files"`, `"folders"`, `"background"`, `".ext"`.
    on: Option<toml::Value>,
    arguments: Option<String>,
}

pub fn changes(registry: &[RegistrySection], env: &[EnvSection], menu: &[ContextMenuSection]) -> Result<SystemChanges> {
    let mut changes = SystemChanges::default();
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
        let value = RegistryValue {
            key: section.key.trim().trim_end_matches('\\').to_string(),
            name: section.name.clone().unwrap_or_default(),
            data,
        };
        system::check_registry_value(&value).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
        if changes.registry.iter().any(|known| known.key.eq_ignore_ascii_case(&value.key) && known.name.eq_ignore_ascii_case(&value.name)) {
            bail!("{at} sets {}\\{} a second time", value.key, value.name);
        }
        changes.registry.push(value);
    }
    for (index, section) in env.iter().enumerate() {
        let at = format!("env[{index}]");
        let var = EnvVar { name: section.name.trim().to_string(), value: section.value.clone() };
        system::check_env_var(&var).map_err(|err| anyhow::anyhow!("{at}: {err}"))?;
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
