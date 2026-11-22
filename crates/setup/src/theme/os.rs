//! The OS "high contrast" setting. On Windows that's the High contrast theme
//! (Settings › Accessibility › Contrast themes). Windows keeps its flags in
//! `HKCU\Control Panel\Accessibility\HighContrast`; bit 0 of `Flags` is
//! HCF_HIGHCONTRASTON. Reading the registry rather than
//! `SystemParametersInfo` keeps the set of `windows` features the same as
//! bach-cli's, which build-release.ps1 insists on.

/// Whether the person turned on a Windows contrast theme.
#[cfg(windows)]
pub fn high_contrast_requested() -> bool {
    if cfg!(test) {
        // A test must give the same answer on every computer.
        return false;
    }
    windows_registry::CURRENT_USER
        .open(r"Control Panel\Accessibility\HighContrast")
        .and_then(|key| key.get_string("Flags"))
        .is_ok_and(|flags| flags_say_on(&flags))
}

/// "Increase contrast" in System Settings › Accessibility › Display. Read once with `defaults`.
#[cfg(target_os = "macos")]
pub fn high_contrast_requested() -> bool {
    use std::sync::OnceLock;
    static ASKED: OnceLock<bool> = OnceLock::new();
    *ASKED.get_or_init(|| {
        !cfg!(test)
            && std::process::Command::new("/usr/bin/defaults")
                .args(["read", "com.apple.universalaccess", "increaseContrast"])
                .output()
                .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1")
    })
}

/// `Flags` is a decimal string; HCF_HIGHCONTRASTON is its lowest bit.
#[cfg_attr(not(windows), allow(dead_code))]
fn flags_say_on(flags: &str) -> bool {
    flags.trim().parse::<u32>().is_ok_and(|flags| flags & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::flags_say_on;

    #[test]
    fn the_lowest_flag_is_the_switch() {
        assert!(flags_say_on("79"), "Windows writes 79 when the theme is on");
        assert!(flags_say_on(" 1 "));
        assert!(!flags_say_on("78"), "78 is the same flags with the switch off");
        assert!(!flags_say_on(""));
        assert!(!flags_say_on("on"));
    }
}
