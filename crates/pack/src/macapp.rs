//! The shape of the app bundles Bach makes for macOS: the Setup app that
//! `bach build` writes and the `Uninstall <app>.app` that setup puts in the
//! install folder. Both are the same runtime program in a bundle, so the
//! builder and the runtime agree on the layout here.
//!
//! ```text
//! Install Hello.app/
//!   Contents/Info.plist
//!   Contents/MacOS/bach-setup          the setup runtime (never modified: code signing)
//!   Contents/Resources/setup.bach      the payload (archive, look, manifest)
//!   Contents/Resources/AppIcon.icns    optional
//! ```

use crate::manifest::{Manifest, Platform};
use crate::paths;

pub use crate::manifest::{MAC_PAYLOAD_IN_BUNDLE as PAYLOAD, MAC_RUNTIME_IN_BUNDLE as RUNTIME};

/// The icon's place inside a bundle.
pub const ICON: &str = "Contents/Resources/AppIcon.icns";

/// The oldest macOS the runtime is built for (GPUI needs Metal; Apple silicon starts at 11).
pub const MIN_SYSTEM: &str = "11.0";

/// What goes into an `Info.plist`.
pub struct Plist<'a> {
    pub bundle_id: &'a str,
    /// The name Finder, the Dock and the menu bar show.
    pub display_name: &'a str,
    pub version: &'a str,
    pub icon: bool,
    pub minimum_system: &'a str,
}

fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// A bundle version needs dotted numbers; an app version like `2.0.0-beta.1` keeps its numeric start.
pub fn bundle_version(version: &str) -> String {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let numeric: String = version.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let numeric = numeric.trim_matches('.').to_string();
    let parts: Vec<&str> = numeric.split('.').filter(|part| !part.is_empty()).take(3).collect();
    if parts.is_empty() {
        "1.0".to_string()
    } else {
        parts.join(".")
    }
}

/// The `Info.plist` text of a Bach bundle.
pub fn info_plist(plist: &Plist) -> String {
    let icon = if plist.icon { "\t<key>CFBundleIconFile</key>\n\t<string>AppIcon</string>\n" } else { "" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>CFBundleDevelopmentRegion</key>\n\t<string>en</string>\n\
         \t<key>CFBundleExecutable</key>\n\t<string>bach-setup</string>\n\
         \t<key>CFBundleIdentifier</key>\n\t<string>{id}</string>\n\
         \t<key>CFBundleInfoDictionaryVersion</key>\n\t<string>6.0</string>\n\
         \t<key>CFBundleName</key>\n\t<string>{name}</string>\n\
         \t<key>CFBundleDisplayName</key>\n\t<string>{name}</string>\n\
         \t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
         \t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n\
         \t<key>CFBundleVersion</key>\n\t<string>{version}</string>\n\
         {icon}\
         \t<key>LSMinimumSystemVersion</key>\n\t<string>{min}</string>\n\
         \t<key>NSHighResolutionCapable</key>\n\t<true/>\n\
         \t<key>NSPrincipalClass</key>\n\t<string>NSApplication</string>\n\
         \t<key>LSApplicationCategoryType</key>\n\t<string>public.app-category.utilities</string>\n\
         </dict>\n</plist>\n",
        id = escape(plist.bundle_id),
        name = escape(plist.display_name),
        version = escape(&bundle_version(plist.version)),
        min = escape(plist.minimum_system),
    )
}

/// The Setup app's name: `Install Hello.app`, which is also what the disk image shows.
pub fn setup_app_name(manifest: &Manifest) -> String {
    format!("Install {}.app", paths::file_name_part(&manifest.app.name, &manifest.app.id))
}

/// The bundle identifier of the uninstaller.
pub fn uninstaller_bundle_id(manifest: &Manifest) -> String {
    let base = manifest
        .mac
        .as_ref()
        .map(|mac| mac.bundle_id.clone())
        .unwrap_or_else(|| format!("app.bach.setup.{}", manifest.app.id.to_lowercase()));
    format!("{base}.uninstall")
}

/// The `Info.plist` of the uninstaller bundle.
pub fn uninstaller_plist(manifest: &Manifest, icon: bool) -> String {
    debug_assert!(manifest.platform == Platform::Macos);
    let id = uninstaller_bundle_id(manifest);
    let name = format!("Uninstall {}", paths::file_name_part(&manifest.app.name, &manifest.app.id));
    let minimum = manifest.mac.as_ref().and_then(|mac| mac.minimum_version.as_deref()).unwrap_or(MIN_SYSTEM);
    info_plist(&Plist { bundle_id: &id, display_name: &name, version: &manifest.app.version, icon, minimum_system: minimum })
}

/// Whether macOS version `have` (like `14.5.1`) is at least `needed` (like `12.0`).
pub fn version_at_least(have: &str, needed: &str) -> bool {
    let parts = |text: &str| -> Vec<u64> { text.split('.').map(|part| part.trim().parse().unwrap_or(0)).collect() };
    let (have, needed) = (parts(have), parts(needed));
    for index in 0..have.len().max(needed.len()) {
        let (a, b) = (have.get(index).copied().unwrap_or(0), needed.get(index).copied().unwrap_or(0));
        if a != b {
            return a > b;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_become_bundle_versions() {
        assert_eq!(bundle_version("1.2.3"), "1.2.3");
        assert_eq!(bundle_version("v2.0.0-beta.1"), "2.0.0");
        assert_eq!(bundle_version("1.2.3.4"), "1.2.3");
        assert_eq!(bundle_version("nightly"), "1.0");
    }

    #[test]
    fn the_plist_is_escaped_and_complete() {
        let text = info_plist(&Plist {
            bundle_id: "com.x.y",
            display_name: "Tom & <Jerry>",
            version: "1.0.0",
            icon: true,
            minimum_system: "12.0",
        });
        assert!(text.contains("<string>Tom &amp; &lt;Jerry&gt;</string>"));
        assert!(text.contains("<key>CFBundleIconFile</key>"));
        assert!(text.contains("<key>LSMinimumSystemVersion</key>\n\t<string>12.0</string>"));
        assert!(!info_plist(&Plist { icon: false, bundle_id: "a.b", display_name: "A", version: "1", minimum_system: "11.0" })
            .contains("CFBundleIconFile"));
    }

    #[test]
    fn os_versions_compare_like_numbers() {
        assert!(version_at_least("14.5", "12.0"));
        assert!(version_at_least("12", "12.0"));
        assert!(version_at_least("12.0.1", "12"));
        assert!(!version_at_least("11.7", "12.0"));
        assert!(!version_at_least("10.15.7", "11"));
        assert!(version_at_least("26.5.2", "26.5.2"));
    }
}
