//! `bach profiles`: the other builds a `bach.toml` describes.

use std::path::Path;

use anyhow::{Context, Result};
use bach_pack::config;
use serde_json::json;

use crate::output;

pub fn command(config_path: &Path) -> Result<()> {
    let text = bach_pack::textfile::read(config_path).with_context(|| format!("couldn't read {}", config_path.display()))?;
    let profiles = config::list_profiles(&text).with_context(|| format!("{} isn't valid", config_path.display()))?;
    let mut listed = Vec::new();
    if profiles.is_empty() {
        say!(
            "{} has no profiles. Add [profiles.NAME] to build another version of the app (see docs/config.md#profiles).",
            config_path.display()
        );
    } else {
        let count = profiles.len();
        say!(
            "{count} {}: `bach build --profile NAME` builds one, `bach build --all-profiles` builds them all.",
            if count == 1 { "profile" } else { "profiles" }
        );
    }
    for profile in &profiles {
        // What the build would write; a profile that doesn't load yet says why instead.
        let loaded = crate::builder::load(config_path, None, Some(&profile.name));
        let (file, problem) = match &loaded {
            Ok(project) => (Some(project.output.strip_prefix(&project.root).unwrap_or(&project.output).display().to_string()), None),
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        say!();
        say!("  {}  ({})", profile.name, profile.kind.name());
        if let Some(description) = &profile.description {
            say!("    {}", crate::diff::clean(description));
        }
        match (&file, &problem) {
            (Some(file), _) => say!("    writes {}", file.replace('\\', "/")),
            (None, Some(problem)) => say!("    can't be built yet: {}", crate::diff::clean(problem)),
            (None, None) => {}
        }
        listed.push(json!({
            "name": profile.name,
            "kind": profile.kind.name(),
            "description": profile.description,
            "output": file.map(|file| file.replace('\\', "/")),
            "problem": problem,
        }));
    }
    output::result("profiles", listed);
    Ok(())
}
