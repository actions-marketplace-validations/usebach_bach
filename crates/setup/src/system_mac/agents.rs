//! The app's own background programs (`[[macos.agents]]`): per-user launch agents that launchd
//! starts at sign-in, keeps alive or runs on a schedule. The Mac's counterpart of Windows services.
//! Setup writes and loads them with the install, replaces them with an update, and takes them
//! away with the app.

use std::path::Path;

use anyhow::Result;
use bach_pack::manifest::{MacAgent, Manifest};

use super::launchd::{self, Agent};

/// `app.bach.agent.<app id>.<name>`.
pub fn label(manifest: &Manifest, agent: &MacAgent) -> String {
    let id: String = manifest.app.id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '-' }).collect();
    format!("app.bach.agent.{id}.{}", agent.name)
}

fn plist_for(manifest: &Manifest, agent: &MacAgent, dir: &Path) -> String {
    let program = bach_pack::paths::under(dir, &agent.file).to_string_lossy().into_owned();
    let mut command = vec![program];
    if let Some(arguments) = &agent.arguments {
        command.extend(super::elevate::split(arguments));
    }
    let label = label(manifest, agent);
    let mut text = Agent {
        label: &label,
        program: command,
        run_at_load: agent.run_at_load,
        interval: agent.interval,
        background: false,
        environment: Vec::new(),
        watch: Vec::new(),
    }
    .plist();
    if agent.keep_alive {
        text = text.replacen("</dict>\n</plist>", "\t<key>KeepAlive</key>\n\t<true/>\n</dict>\n</plist>", 1);
    }
    text
}

/// Writes (and loads) the agents of `manifest`, and takes away those `previous` had and `manifest` doesn't.
pub fn apply(manifest: &Manifest, previous: Option<&Manifest>, dir: &Path) -> Result<()> {
    if let Some(previous) = previous {
        for old in &previous.agents {
            if !manifest.agents.iter().any(|now| now.name == old.name) {
                launchd::remove(&label(previous, old))?;
            }
        }
    }
    for agent in &manifest.agents {
        let label = label(manifest, agent);
        // An update replaces the program: the old process is stopped and the new one started.
        launchd::unload(&label);
        let target = launchd::path(&label)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = target.with_extension("plist.new");
        std::fs::write(&partial, plist_for(manifest, agent, dir))?;
        std::fs::rename(&partial, &target)?;
        launchd::load(&target);
    }
    Ok(())
}

/// Stops and deletes every agent of the app. Removing never stops for a failure.
pub fn remove(manifest: &Manifest) {
    for agent in &manifest.agents {
        let _ = launchd::remove(&label(manifest, agent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        let json = r#"{"format":1,"builder":"t","platform":"macos","app":{"id":"Hello","name":"Hello","version":"1.0.0","publisher":"P","executable":"Hello.app"},
            "install":{"folder":"Hello","desktop_shortcut":true},"uninstall":{"data":[],"data_description":null},
            "look":{"theme":"system","accent":null,"logo":null},
            "files":[{"path":"bin/helper","size":1,"mode":493}],
            "agents":[{"name":"sync","file":"bin/helper","arguments":"--watch \"a b\"","keep_alive":true,"interval":600}]}"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn an_agent_runs_its_program_from_the_install_folder() {
        let manifest = manifest();
        let text = plist_for(&manifest, &manifest.agents[0], Path::new("/Applications/Hello"));
        assert!(text.contains("<string>app.bach.agent.Hello.sync</string>"), "{text}");
        assert!(text.contains("<string>/Applications/Hello/bin/helper</string>"));
        assert!(text.contains("<string>--watch</string>") && text.contains("<string>a b</string>"));
        assert!(text.contains("<key>StartInterval</key>\n\t<integer>600</integer>") && text.contains("<key>KeepAlive</key>"));
        assert!(text.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn agents_come_with_an_install_and_go_with_the_app() {
        let home = std::env::temp_dir().join(format!("bach-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        crate::system::with_home(&home, || {
            let manifest = manifest();
            apply(&manifest, None, Path::new("/Applications/Hello")).unwrap();
            let file = home.join("Library/LaunchAgents/app.bach.agent.Hello.sync.plist");
            assert!(file.is_file());
            // An update that drops the agent takes it away.
            let mut next = manifest.clone();
            next.agents.clear();
            apply(&next, Some(&manifest), Path::new("/Applications/Hello")).unwrap();
            assert!(!file.exists());
            apply(&manifest, None, Path::new("/Applications/Hello")).unwrap();
            remove(&manifest);
            assert!(!file.exists());
        });
        std::fs::remove_dir_all(&home).unwrap();
    }
}
