//! launchd agents: the small property lists in `~/Library/LaunchAgents` that start a
//! program when the user signs in or on a schedule. Used for "Open at login" and
//! for the background update task.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// The text of a launch agent's property list.
pub struct Agent<'a> {
    pub label: &'a str,
    pub program: Vec<String>,
    pub run_at_load: bool,
    /// Run again every this many seconds.
    pub interval: Option<u32>,
    /// Lowers the program's priority, for background work.
    pub background: bool,
    /// Environment variables the program starts with.
    pub environment: Vec<(String, String)>,
    /// Folders that start the program whenever something in them changes.
    pub watch: Vec<PathBuf>,
}

fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

impl Agent<'_> {
    pub fn plist(&self) -> String {
        let arguments: String = self.program.iter().map(|argument| format!("\t\t<string>{}</string>\n", escape(argument))).collect();
        let mut text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>{}</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n{arguments}\t</array>\n",
            escape(self.label)
        );
        if self.run_at_load {
            text.push_str("\t<key>RunAtLoad</key>\n\t<true/>\n");
        }
        if let Some(seconds) = self.interval {
            text.push_str(&format!("\t<key>StartInterval</key>\n\t<integer>{seconds}</integer>\n"));
        }
        if self.background {
            text.push_str("\t<key>ProcessType</key>\n\t<string>Background</string>\n\t<key>LowPriorityIO</key>\n\t<true/>\n");
        }
        if !self.environment.is_empty() {
            text.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
            for (name, value) in &self.environment {
                text.push_str(&format!("\t\t<key>{}</key>\n\t\t<string>{}</string>\n", escape(name), escape(value)));
            }
            text.push_str("\t</dict>\n");
        }
        if !self.watch.is_empty() {
            text.push_str("\t<key>WatchPaths</key>\n\t<array>\n");
            for folder in &self.watch {
                text.push_str(&format!("\t\t<string>{}</string>\n", escape(&folder.to_string_lossy())));
            }
            text.push_str("\t</array>\n\t<key>ThrottleInterval</key>\n\t<integer>10</integer>\n");
        }
        text.push_str("</dict>\n</plist>\n");
        text
    }
}

/// `~/Library/LaunchAgents/<label>.plist`.
pub fn path(label: &str) -> Result<PathBuf> {
    Ok(super::home()?.join("Library").join("LaunchAgents").join(format!("{label}.plist")))
}

/// Writes the agent's file. launchd loads it at the next sign-in; [`load`] starts it now.
pub fn write(agent: &Agent) -> Result<PathBuf> {
    let target = path(agent.label)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
    }
    let partial = target.with_extension("plist.new");
    std::fs::write(&partial, agent.plist()).with_context(|| format!("couldn't write {}", partial.display()))?;
    std::fs::rename(&partial, &target).with_context(|| format!("couldn't write {}", target.display()))?;
    Ok(target)
}

fn domain() -> String {
    // SAFETY: getuid has no arguments and cannot fail.
    format!("gui/{}", unsafe { libc::getuid() })
}

/// Whether tests keep launchd out of it (`BACH_NO_LAUNCHCTL`): agents are written, never loaded.
fn skip_launchctl() -> bool {
    std::env::var_os("BACH_NO_LAUNCHCTL").is_some()
}

/// Starts the agent in this user's session now (a failure only means it starts at the next sign-in).
pub fn load(file: &Path) {
    if skip_launchctl() {
        return;
    }
    let _ = Command::new("/bin/launchctl").arg("bootstrap").arg(domain()).arg(file).output();
}

/// Stops the agent and forgets it.
pub fn unload(label: &str) {
    if skip_launchctl() {
        return;
    }
    let _ = Command::new("/bin/launchctl").arg("bootout").arg(format!("{}/{label}", domain())).output();
}

/// Deletes the agent's file. Fine when there is none.
pub fn remove(label: &str) -> Result<()> {
    unload(label);
    match std::fs::remove_file(path(label)?) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_names_the_program_and_is_escaped() {
        let agent = Agent {
            label: "app.bach.update.Tom",
            program: vec!["/Applications/Tom & Jerry/Uninstall Tom.app/Contents/MacOS/bach-setup".into(), "--auto-update".into()],
            run_at_load: false,
            interval: Some(14_400),
            background: true,
            environment: vec![("A".into(), "x & y".into())],
            watch: vec![PathBuf::from("/Applications/Tom")],
        };
        let text = agent.plist();
        assert!(text.contains("<string>/Applications/Tom &amp; Jerry/Uninstall Tom.app/Contents/MacOS/bach-setup</string>"), "{text}");
        assert!(text.contains("<string>--auto-update</string>"));
        assert!(text.contains("<key>StartInterval</key>\n\t<integer>14400</integer>"));
        assert!(!text.contains("RunAtLoad"));
        assert!(text.contains("<key>A</key>\n\t\t<string>x &amp; y</string>"), "{text}");
        assert!(text.contains("<key>WatchPaths</key>\n\t<array>\n\t\t<string>/Applications/Tom</string>"), "{text}");
        let login = Agent {
            label: "x.y",
            program: vec!["/usr/bin/open".into()],
            run_at_load: true,
            interval: None,
            background: false,
            environment: Vec::new(),
            watch: Vec::new(),
        };
        assert!(login.plist().contains("<key>RunAtLoad</key>\n\t<true/>") && !login.plist().contains("StartInterval"));
    }
}
