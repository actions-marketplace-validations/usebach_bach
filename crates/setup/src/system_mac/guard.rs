//! Moving the app to the Trash is how a Mac user removes an app, and nothing in
//! the app can notice that. So setup leaves a small watcher behind: a launch agent
//! that wakes up when the install folder (or the folder around it) changes and, when
//! the app is gone, cleans up what setup added around it: the links, the launch agents,
//! the PATH block, the install record and the uninstaller. The user's data is never touched.
//!
//! The watcher runs a short shell script and not the uninstaller, because when the user
//! trashes the whole folder the uninstaller is gone with it; the script then does the
//! cleanup itself. When the uninstaller is still there it does the work (`--cleanup-if-removed`),
//! the same removal as Uninstall, without the data.

use std::path::{Path, PathBuf};

use anyhow::Result;
use bach_pack::manifest::Manifest;

use super::launchd;

/// What the watcher runs. Everything it needs arrives in environment variables, so no path
/// or name is ever part of the script's text.
const SCRIPT: &str = r##"
[ -e "$BACH_APP" ] && exit 0
# A move that is still going on, or an update swapping files, isn't a removal.
sleep 5
[ -e "$BACH_APP" ] && exit 0
if [ -x "$BACH_UNINSTALLER" ]; then
  exec "$BACH_UNINSTALLER" --cleanup-if-removed
fi
# The whole folder is gone: undo what setup added around it.
printf '%s\n' "$BACH_LINKS" | while IFS= read -r link; do
  [ -n "$link" ] && [ -L "$link" ] && [ ! -e "$link" ] && rm -f "$link"
done
for label in $BACH_LABELS; do
  /bin/launchctl bootout "gui/$(id -u)/$label" 2>/dev/null
  rm -f "$HOME/Library/LaunchAgents/$label.plist"
done
[ -z "$BACH_RECORD" ] || rm -f "$BACH_RECORD"
[ -z "$BACH_UNINSTALLER_DIR" ] || rm -rf "$BACH_UNINSTALLER_DIR"
for profile in "$HOME/.zprofile" "$HOME/.bash_profile"; do
  [ -f "$profile" ] || continue
  awk -v first="# >>> bach path $BACH_PATH_FOLDER >>>" -v last="# <<< bach path $BACH_PATH_FOLDER <<<" '
    $0 == first { skip = 1; next }
    skip && $0 == last { skip = 0; next }
    !skip { print }' "$profile" > "$profile.bach-new" && cat "$profile.bach-new" > "$profile"
  rm -f "$profile.bach-new"
done
rm -f "$BACH_GUARD_PLIST"
exit 0
"##;

pub fn label(manifest: &Manifest) -> String {
    let id: String = manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '-' }).collect();
    format!("app.bach.guard.{id}")
}

/// The launch agents setup keeps for this app, which the watcher takes away with it.
fn companion_labels(manifest: &Manifest) -> Vec<String> {
    let id: String = manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '-' }).collect();
    let mut labels = vec![format!("app.bach.login.{id}"), format!("app.bach.update.{id}")];
    labels.extend(manifest.agents.iter().map(|agent| super::agents::label(manifest, agent)));
    labels
}

/// The environment the script reads, and the folders it watches.
pub struct Plan {
    pub environment: Vec<(String, String)>,
    pub watch: Vec<PathBuf>,
}

/// What the watcher for `manifest` installed in `dir` is told.
pub fn plan(manifest: &Manifest, dir: &Path) -> Result<Plan> {
    let uninstaller = super::uninstaller_bundle(manifest).join(bach_pack::macapp::RUNTIME);
    let links: Vec<String> = [super::start_menu_shortcut(manifest).ok(), super::desktop_shortcut(manifest).ok()]
        .into_iter()
        .flatten()
        .map(|link| link.to_string_lossy().into_owned())
        .collect();
    let record = super::support_dir()?.join("installs").join(format!("{}.json", bach_pack::paths::safe_dir_name(&manifest.app.id)));
    let guard_plist = launchd::path(&label(manifest))?;
    let environment = vec![
        ("BACH_APP".to_string(), manifest.executable_in(dir).to_string_lossy().into_owned()),
        ("BACH_UNINSTALLER".to_string(), uninstaller.to_string_lossy().into_owned()),
        ("BACH_LINKS".to_string(), links.join("\n")),
        ("BACH_LABELS".to_string(), companion_labels(manifest).join(" ")),
        ("BACH_RECORD".to_string(), record.to_string_lossy().into_owned()),
        ("BACH_UNINSTALLER_DIR".to_string(), super::uninstaller_folder(manifest)?.to_string_lossy().into_owned()),
        ("BACH_PATH_FOLDER".to_string(), manifest.path_folder_in(dir).to_string_lossy().into_owned()),
        ("BACH_GUARD_PLIST".to_string(), guard_plist.to_string_lossy().into_owned()),
    ];
    // The folder of an app with one, and the folder around it (for when the whole folder is moved).
    // An app that sits directly in Applications watches Applications only.
    let mut watch = vec![dir.to_path_buf()];
    if !manifest.install.folder.is_empty() {
        watch.extend(dir.parent().map(Path::to_path_buf));
    }
    Ok(Plan { environment, watch })
}

/// Leaves the watcher in `~/Library/LaunchAgents` (loaded now, and at every sign-in).
pub fn install(manifest: &Manifest, dir: &Path) -> Result<()> {
    let plan = plan(manifest, dir)?;
    let label = label(manifest);
    let agent = launchd::Agent {
        label: &label,
        program: vec!["/bin/sh".into(), "-c".into(), SCRIPT.into()],
        run_at_load: false,
        interval: None,
        background: true,
        environment: plan.environment,
        watch: plan.watch,
    };
    launchd::unload(&label);
    let file = launchd::write(&agent)?;
    launchd::load(&file);
    Ok(())
}

/// Takes the watcher away. Fine when there is none.
pub fn remove(manifest: &Manifest) -> Result<()> {
    launchd::remove(&label(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn manifest() -> Manifest {
        let json = r#"{"format":1,"builder":"t","platform":"macos","app":{"id":"Guarded","name":"Guarded","version":"1.0.0","publisher":"P","executable":"Guarded.app"},
            "install":{"folder":"Guarded","desktop_shortcut":true},"uninstall":{"data":[],"data_description":null},
            "look":{"theme":"system","accent":null,"logo":null},"files":[{"path":"Guarded.app/Contents/x","size":1}]}"#;
        serde_json::from_str(json).unwrap()
    }

    fn run_script(environment: &[(String, String)], home: &Path) -> std::process::ExitStatus {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(SCRIPT.replace("sleep 5", "true")).env("HOME", home).env("BACH_NO_LAUNCHCTL", "1");
        for (name, value) in environment {
            command.env(name, value);
        }
        command.status().unwrap()
    }

    #[test]
    fn a_trashed_folder_leaves_nothing_behind_but_the_users_data() {
        let base = std::env::temp_dir().join(format!("bach-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let dir = home.join("Applications/Guarded");
        std::fs::create_dir_all(home.join("Applications")).unwrap();
        std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
        std::fs::create_dir_all(home.join("Library/Application Support/Bach/installs")).unwrap();
        std::fs::create_dir_all(home.join("Library/Application Support/Guarded")).unwrap();
        std::fs::write(home.join("Library/Application Support/Guarded/data.txt"), b"mine").unwrap();
        let mut manifest = manifest();
        manifest.install.start_menu_shortcut = bach_pack::manifest::Choice::Always;
        crate::system::with_home(&home, || {
            let plan = plan(&manifest, &dir).unwrap();
            // What setup left: a link to the (now trashed) app, agents, the record, a PATH block.
            let link = home.join("Applications/Guarded.app");
            std::os::unix::fs::symlink(dir.join("Guarded.app"), &link).unwrap();
            for label in ["app.bach.login.Guarded", "app.bach.update.Guarded", "app.bach.guard.Guarded"] {
                std::fs::write(home.join(format!("Library/LaunchAgents/{label}.plist")), b"<plist/>").unwrap();
            }
            let record = home.join("Library/Application Support/Bach/installs/Guarded.json");
            std::fs::write(&record, b"{}").unwrap();
            let folder = dir.to_string_lossy().into_owned();
            std::fs::write(
                home.join(".zprofile"),
                format!(
                    "export A=1\n# >>> bach path {folder} >>>\nexport PATH=\"{folder}:$PATH\"\n# <<< bach path {folder} <<<\nexport B=2\n"
                ),
            )
            .unwrap();

            // While the app is there, nothing happens.
            std::fs::create_dir_all(dir.join("Guarded.app")).unwrap();
            assert!(run_script(&plan.environment, &home).success());
            assert!(link.exists() && record.exists());
            // The folder is moved to the Trash: everything setup added goes, the data stays.
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(run_script(&plan.environment, &home).success());
            assert!(std::fs::symlink_metadata(&link).is_err(), "the dangling link is gone");
            assert!(!record.exists());
            for label in ["app.bach.login.Guarded", "app.bach.update.Guarded", "app.bach.guard.Guarded"] {
                assert!(!home.join(format!("Library/LaunchAgents/{label}.plist")).exists(), "{label}");
            }
            assert_eq!(std::fs::read_to_string(home.join(".zprofile")).unwrap(), "export A=1\nexport B=2\n");
            assert!(home.join("Library/Application Support/Guarded/data.txt").exists(), "the user's data is never touched");
        });
        std::fs::remove_dir_all(&base).unwrap();
    }
}
