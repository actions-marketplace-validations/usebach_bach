//! Files that must not ship by accident: credentials, signing keys, version
//! control data and the project's own `bach.toml`.
//!
//! Pointing `files.source` at a project folder instead of a build folder is an
//! easy mistake, and a published key is burnt for good. So a file that looks
//! like a secret stops the build until `files.allow_secrets` names it, and
//! files that are merely clutter (debug symbols, logs, `node_modules`) are
//! reported as warnings.

use anyhow::{bail, Result};

use crate::archive::Source;
use crate::paths;

/// File name patterns (matched on the name, anywhere in the tree) with what
/// makes each one a secret.
const SECRET_NAMES: &[(&str, &str)] = &[
    (".env", "an environment file with credentials"),
    (".env.*", "an environment file with credentials"),
    ("*.pfx", "a signing certificate with its private key"),
    ("*.p12", "a signing certificate with its private key"),
    ("*.pem", "a key or certificate"),
    ("*.key", "a private key"),
    ("*.snk", "a strong-name signing key"),
    ("*.jks", "a Java key store"),
    ("*.keystore", "a key store"),
    ("*.kdbx", "a password database"),
    ("id_rsa*", "an SSH private key"),
    ("id_dsa*", "an SSH private key"),
    ("id_ecdsa*", "an SSH private key"),
    ("id_ed25519*", "an SSH private key"),
    (".git-credentials", "stored Git credentials"),
    (".npmrc", "npm settings that often hold a token"),
    (".pypirc", "PyPI credentials"),
    (".netrc", "stored logins"),
    (".aws-credentials", "AWS credentials"),
    ("secrets.json", "a file named like a secret"),
    ("bach.toml", "the project's own bach.toml, with signing commands and feed addresses"),
    ("bach-*.toml", "a bach.toml of this project, with signing commands and feed addresses"),
];

/// Folder names whose whole contents are secret.
const SECRET_FOLDERS: &[(&str, &str)] = &[
    (".git", "Git history and settings"),
    (".ssh", "SSH keys"),
    (".aws", "AWS credentials"),
    (".gnupg", "GPG keys"),
];

/// Names that only add weight: warned about, never refused.
const CLUTTER_NAMES: &[&str] = &["*.pdb", "*.log", ".DS_Store", "Thumbs.db"];
const CLUTTER_FOLDERS: &[&str] = &["node_modules", "target", "__pycache__"];

/// Why a packaged path looks like a secret, if it does.
pub fn secret_reason(packaged: &str) -> Option<&'static str> {
    let mut parts: Vec<&str> = packaged.split('/').collect();
    let name = parts.pop().unwrap_or(packaged);
    for folder in parts {
        if let Some((_, reason)) = SECRET_FOLDERS.iter().find(|(secret, _)| folder.eq_ignore_ascii_case(secret)) {
            return Some(reason);
        }
    }
    SECRET_NAMES.iter().find(|(pattern, _)| paths::matches(pattern, name)).map(|(_, reason)| *reason)
}

/// Whether a path is clutter that rarely belongs in an installer.
pub fn is_clutter(packaged: &str) -> bool {
    let mut parts: Vec<&str> = packaged.split('/').collect();
    let name = parts.pop().unwrap_or(packaged);
    parts.iter().any(|folder| CLUTTER_FOLDERS.iter().any(|clutter| folder.eq_ignore_ascii_case(clutter)))
        || CLUTTER_NAMES.iter().any(|pattern| paths::matches(pattern, name))
}

/// Refuses a source folder that holds a secret nobody allowed, and warns
/// about clutter.
pub fn check(sources: &[Source], allowed: &[String], warnings: &mut Vec<String>) -> Result<()> {
    let found: Vec<(&str, &str)> = sources
        .iter()
        .filter(|source| !allowed.iter().any(|pattern| paths::matches(pattern, &source.packaged)))
        .filter_map(|source| secret_reason(&source.packaged).map(|reason| (source.packaged.as_str(), reason)))
        .collect();
    if let Some((first, reason)) = found.first() {
        let more = match found.len() - 1 {
            0 => String::new(),
            n => format!(" (and {n} more like it)"),
        };
        bail!(
            "files.source holds \"{first}\", {reason}{more}; leave it out with files.exclude, \
             or package it on purpose with files.allow_secrets = [\"{first}\"]"
        );
    }
    let clutter: Vec<&str> =
        sources.iter().map(|source| source.packaged.as_str()).filter(|path| is_clutter(path)).collect();
    if let Some(first) = clutter.first() {
        warnings.push(format!(
            "files.source holds {} file(s) that rarely belong in an installer, such as \"{first}\"; \
             leave them out with files.exclude (for example \"*.pdb\", \"*.log\", \"node_modules/**\")",
            clutter.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sources(paths: &[&str]) -> Vec<Source> {
        paths.iter().map(|path| Source { packaged: path.to_string(), path: PathBuf::from(path), size: 1 }).collect()
    }

    #[test]
    fn every_known_secret_is_found() {
        for path in [
            ".env",
            ".env.production",
            "cfg/.ENV",
            "cert.pfx",
            "signing.p12",
            "server.pem",
            "private.key",
            "keys/Private.KEY",
            "id_rsa",
            "id_rsa.pub",
            "id_ed25519",
            "id_ecdsa",
            ".ssh/id_ed25519",
            "home/.SSH/config",
            ".git/config",
            "sub/.git/HEAD",
            ".git-credentials",
            ".npmrc",
            ".pypirc",
            ".aws/credentials",
            ".aws-credentials",
            "secrets.json",
            "vault.kdbx",
            "strong.snk",
            "bach.toml",
            "Bach.TOML",
            "sub/bach-1.0.0.toml",
        ] {
            assert!(secret_reason(path).is_some(), "{path} should be a secret");
        }
    }

    #[test]
    fn ordinary_files_are_not_secrets() {
        for path in ["app.exe", "data/environment.txt", "keyboard.dll", "readme.md", "env/config.json", "git/readme.txt", "monkey.png"] {
            assert!(secret_reason(path).is_none(), "{path} should be fine");
        }
    }

    #[test]
    fn a_secret_stops_the_build_until_it_is_allowed() {
        let found = sources(&["app.exe", ".env", "id_rsa"]);
        let err = check(&found, &[], &mut Vec::new()).unwrap_err().to_string();
        assert!(err.contains("files.source") && err.contains("\".env\"") && err.contains("1 more") && err.contains("files.allow_secrets"), "{err}");
        assert!(check(&found, &[".env".into(), "id_rsa".into()], &mut Vec::new()).is_ok());
        assert!(check(&found, &["*".into()], &mut Vec::new()).is_ok(), "a pattern covers many files");
        assert!(check(&found, &[".ENV".into()], &mut Vec::new()).is_err(), "id_rsa is still refused");
    }

    #[test]
    fn clutter_is_a_warning_not_an_error() {
        let found = sources(&["app.exe", "app.pdb", "debug.log", "node_modules/x/index.js", "target/a.o", "Thumbs.db", ".DS_Store"]);
        let mut warnings = Vec::new();
        assert!(check(&found, &[], &mut warnings).is_ok());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("6 file(s)"), "{}", warnings[0]);
        let mut clean = Vec::new();
        check(&sources(&["app.exe", "data/readme.txt"]), &[], &mut clean).unwrap();
        assert!(clean.is_empty());
    }
}
