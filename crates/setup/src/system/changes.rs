//! `[[registry]]`, `[[env]]` and `[[context_menu]]`: registry values,
//! environment variables and Explorer's right-click entries.
//!
//! Setup only ever undoes what its own manifest lists, and only while Windows
//! still holds what setup wrote: a registry value or environment variable
//! someone else changed since stays as they left it, and a menu key without
//! this app's `BachApp` mark is never touched.

use std::path::Path;

use anyhow::{Context, Result};
use bach_pack::manifest::system::expand_app;
use bach_pack::manifest::{ContextMenuEntry, ContextTarget, EnvVar, Manifest, RegistryData, RegistryValue};
use windows_registry::{Key, CURRENT_USER};

const CLASSES: &str = r"Software\Classes";
const MARK: &str = "BachApp";

/// Where the changes go: the per-user hive, or the machine's.
pub struct Where<'a> {
    pub hive: &'a Key,
    /// The key of the environment variables, below the hive.
    pub environment: &'a str,
}

/// Where this app's changes go: the user's registry, or the computer's for a
/// machine-wide install.
pub fn at(manifest: &Manifest) -> Where<'static> {
    let (hive, environment) = super::env_path::location(manifest.install.scope);
    Where { hive, environment }
}

/// Writes everything the manifest lists, after taking back what the
/// previous version made and this one doesn't.
pub fn apply(at: &Where, manifest: &Manifest, previous: Option<&Manifest>, dir: &Path, exe: &Path) -> Result<()> {
    if let Some(previous) = previous {
        undo_stale(at, manifest, previous, dir);
    }
    let system = &manifest.system;
    for value in &system.registry {
        write_value(at, value, dir).with_context(|| format!("couldn't write {}", value.key))?;
    }
    let mut env_changed = false;
    for var in &system.environment {
        write_env(at, var, dir).with_context(|| format!("couldn't set {}", var.name))?;
        env_changed = true;
    }
    if env_changed {
        super::env_path::broadcast();
    }
    for entry in &system.context_menu {
        register_menu(at, manifest, entry, exe).with_context(|| format!("couldn't add \"{}\" to the right-click menu", entry.name))?;
    }
    Ok(())
}

/// Takes back everything the manifest lists.
pub fn undo(at: &Where, manifest: &Manifest, dir: &Path) {
    let system = &manifest.system;
    for value in &system.registry {
        remove_value(at, value, dir);
    }
    let mut env_changed = false;
    for var in &system.environment {
        env_changed |= remove_env(at, var, dir);
    }
    if env_changed {
        super::env_path::broadcast();
    }
    for entry in &system.context_menu {
        unregister_menu(at, manifest, entry, &entry.on);
    }
}

/// What an update takes back: what the old version made and the new one doesn't.
fn undo_stale(at: &Where, manifest: &Manifest, previous: &Manifest, dir: &Path) {
    let now = &manifest.system;
    for old in &previous.system.registry {
        if !now.registry.iter().any(|new| same_value(new, old)) {
            remove_value(at, old, dir);
        }
    }
    let mut env_changed = false;
    for old in &previous.system.environment {
        if !now.environment.iter().any(|new| new.name.eq_ignore_ascii_case(&old.name)) {
            env_changed |= remove_env(at, old, dir);
        }
    }
    if env_changed {
        super::env_path::broadcast();
    }
    for old in &previous.system.context_menu {
        let verb = old.verb(&manifest.app.id);
        let kept: Vec<&ContextTarget> = now
            .context_menu
            .iter()
            .filter(|new| new.verb(&manifest.app.id) == verb)
            .flat_map(|new| new.on.iter())
            .collect();
        let gone: Vec<ContextTarget> = old.on.iter().filter(|target| !kept.contains(target)).cloned().collect();
        unregister_menu(at, manifest, old, &gone);
    }
}

fn same_value(a: &RegistryValue, b: &RegistryValue) -> bool {
    a.key.eq_ignore_ascii_case(&b.key) && a.name.eq_ignore_ascii_case(&b.name)
}

fn write_value(at: &Where, value: &RegistryValue, dir: &Path) -> Result<()> {
    let key = at.hive.create(&value.key)?;
    match &value.data {
        RegistryData::String(text) => key.set_string(&value.name, expand_app(text, dir))?,
        RegistryData::Expand(text) => key.set_expand_string(&value.name, expand_app(text, dir))?,
        RegistryData::Dword(number) => key.set_u32(&value.name, *number)?,
    }
    Ok(())
}

/// Whether the registry still holds exactly what setup wrote.
fn holds(key: &Key, value: &RegistryValue, dir: &Path) -> bool {
    match &value.data {
        RegistryData::String(text) | RegistryData::Expand(text) => {
            key.get_string(&value.name).is_ok_and(|current| current == expand_app(text, dir))
        }
        RegistryData::Dword(number) => key.get_u32(&value.name).is_ok_and(|current| current == *number),
    }
}

fn remove_value(at: &Where, value: &RegistryValue, dir: &Path) {
    let Ok(key) = at.hive.options().read().write().open(&value.key) else {
        return;
    };
    if holds(&key, value, dir) {
        let _ = key.remove_value(&value.name);
    }
    drop(key);
    prune(at, &value.key);
}

/// Removes `path` and the keys above it while they're empty, but never
/// `Software` itself and never a key that still holds anything.
fn prune(at: &Where, path: &str) {
    let mut parts: Vec<&str> = path.split('\\').collect();
    while parts.len() >= 2 {
        let current = parts.join("\\");
        let empty = at.hive.open(&current).is_ok_and(|key| {
            key.keys().is_ok_and(|mut keys| keys.next().is_none()) && key.values().is_ok_and(|mut values| values.next().is_none())
        });
        if !empty || at.hive.remove_tree(&current).is_err() {
            return;
        }
        parts.pop();
    }
}

fn write_env(at: &Where, var: &EnvVar, dir: &Path) -> Result<()> {
    let value = expand_app(&var.value, dir);
    let key = at.hive.create(at.environment)?;
    if value.contains('%') {
        key.set_expand_string(&var.name, &value)?;
    } else {
        key.set_string(&var.name, &value)?;
    }
    Ok(())
}

/// Removes the variable if it still holds what setup set; true when it did.
fn remove_env(at: &Where, var: &EnvVar, dir: &Path) -> bool {
    let Ok(key) = at.hive.options().read().write().open(at.environment) else {
        return false;
    };
    key.get_string(&var.name).is_ok_and(|current| current == expand_app(&var.value, dir)) && key.remove_value(&var.name).is_ok()
}

fn menu_key(entry: &ContextMenuEntry, app_id: &str, target: &ContextTarget) -> String {
    format!(r"{CLASSES}\{}\shell\{}", target.class(), entry.verb(app_id))
}

fn register_menu(at: &Where, manifest: &Manifest, entry: &ContextMenuEntry, exe: &Path) -> Result<()> {
    for target in &entry.on {
        let path = menu_key(entry, &manifest.app.id, target);
        // A verb of the same name that isn't ours is somebody else's.
        if at.hive.open(&path).is_ok_and(|key| key.get_string(MARK).map_or(true, |id| id != manifest.app.id)) {
            crate::log::line(&format!("The menu entry {path} belongs to another program, so setup left it."));
            continue;
        }
        let key = at.hive.create(&path)?;
        key.set_string(MARK, &manifest.app.id)?;
        key.set_string("", &entry.name)?;
        key.set_string("Icon", format!("{},0", exe.display()))?;
        let arguments = entry.arguments.as_deref().unwrap_or_else(|| target.default_argument());
        key.create("command")?.set_string("", format!("\"{}\" {arguments}", exe.display()))?;
    }
    Ok(())
}

fn unregister_menu(at: &Where, manifest: &Manifest, entry: &ContextMenuEntry, targets: &[ContextTarget]) {
    for target in targets {
        let path = menu_key(entry, &manifest.app.id, target);
        let ours = at.hive.open(&path).is_ok_and(|key| key.get_string(MARK).is_ok_and(|id| id == manifest.app.id));
        if ours {
            let _ = at.hive.remove_tree(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bach_pack::manifest::SystemChanges;

    fn manifest(id: &str, system: SystemChanges) -> Manifest {
        let mut manifest = crate::package::Package::sample().manifest;
        manifest.app.id = id.into();
        manifest.system = system;
        manifest
    }

    fn scratch(tag: &str) -> String {
        format!(r"Software\BachTest-{tag}-{}", std::process::id())
    }

    #[test]
    fn registry_values_and_variables_come_and_go() {
        let base = scratch("changes");
        let env = format!(r"{base}\Env");
        let at = Where { hive: CURRENT_USER, environment: &env };
        let dir = Path::new(r"C:\Apps\Hello");
        let exe = dir.join("hello.exe");
        let system = SystemChanges {
            registry: vec![
                RegistryValue { key: format!(r"{base}\App"), name: "Dir".into(), data: RegistryData::Expand(r"{app}\bin".into()) },
                RegistryValue { key: format!(r"{base}\App"), name: "Level".into(), data: RegistryData::Dword(3) },
                RegistryValue { key: format!(r"{base}\Other"), name: String::new(), data: RegistryData::String("x".into()) },
            ],
            environment: vec![EnvVar { name: "BACH_TEST_HOME".into(), value: "{app}".into() }],
            ..SystemChanges::default()
        };
        let manifest = manifest("ChangesTest", system.clone());

        apply(&at, &manifest, None, dir, &exe).unwrap();
        let app = CURRENT_USER.options().read().write().open(format!(r"{base}\App")).unwrap();
        assert_eq!(app.get_string("Dir").unwrap(), r"C:\Apps\Hello\bin");
        assert_eq!(app.get_u32("Level").unwrap(), 3);
        assert_eq!(CURRENT_USER.open(&env).unwrap().get_string("BACH_TEST_HOME").unwrap(), r"C:\Apps\Hello");

        // An update that drops a value takes it away, and keeps the rest.
        let mut newer = manifest.clone();
        newer.system.registry.remove(1);
        newer.system.environment.clear();
        apply(&at, &newer, Some(&manifest), dir, &exe).unwrap();
        assert!(app.get_u32("Level").is_err());
        assert!(app.get_string("Dir").is_ok());
        assert!(CURRENT_USER.open(&env).unwrap().get_string("BACH_TEST_HOME").is_err());

        // Someone else changed a value: uninstalling leaves theirs alone.
        app.set_string("Dir", "theirs").unwrap();
        undo(&at, &newer, dir);
        assert_eq!(app.get_string("Dir").unwrap(), "theirs");
        assert!(CURRENT_USER.open(format!(r"{base}\Other")).is_err(), "the emptied key is gone");

        app.remove_value("Dir").unwrap();
        drop(app);
        undo(&at, &newer, dir);
        assert!(CURRENT_USER.open(format!(r"{base}\App")).is_err());
        let _ = CURRENT_USER.remove_tree(&base);
    }

    #[test]
    fn menu_entries_are_ours_alone() {
        let tag = std::process::id();
        let extension = format!(".bt{tag}");
        let at = Where { hive: CURRENT_USER, environment: "Environment" };
        let dir = Path::new(r"C:\Apps\Hello");
        let exe = dir.join("hello.exe");
        let entry = ContextMenuEntry {
            name: "Open with Hello".into(),
            on: vec![ContextTarget::Extension(extension.clone())],
            arguments: None,
        };
        let ours = manifest("MenuTest", SystemChanges { context_menu: vec![entry.clone()], ..SystemChanges::default() });
        let theirs = manifest("MenuOther", SystemChanges { context_menu: vec![entry.clone()], ..SystemChanges::default() });
        let path = format!(r"{CLASSES}\SystemFileAssociations\{extension}\shell\{}", entry.verb("MenuTest"));

        apply(&at, &ours, None, dir, &exe).unwrap();
        let key = CURRENT_USER.open(&path).unwrap();
        assert_eq!(key.get_string("").unwrap(), "Open with Hello");
        assert_eq!(CURRENT_USER.open(format!(r"{path}\command")).unwrap().get_string("").unwrap(), r#""C:\Apps\Hello\hello.exe" "%1""#);

        undo(&at, &theirs, dir);
        assert!(CURRENT_USER.open(&path).is_ok(), "only the owner removes it");
        undo(&at, &ours, dir);
        assert!(CURRENT_USER.open(&path).is_err());
        let _ = CURRENT_USER.remove_tree(format!(r"{CLASSES}\SystemFileAssociations\{extension}"));
    }
}
