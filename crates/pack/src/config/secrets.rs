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
    ("*.env", "an environment file with credentials"),
    ("*.ppk", "a PuTTY private key"),
    ("*.p8", "an Apple or APNs private key"),
    ("secring.gpg", "a GPG secret keyring"),
    ("trustdb.gpg", "a GPG trust database"),
    ("*.tfstate", "Terraform state, which holds credentials in the clear"),
    ("*.tfstate.*", "Terraform state, which holds credentials in the clear"),
    ("*.tfvars", "Terraform variables, often credentials"),
    ("credentials", "a credentials file"),
    ("credentials.json", "a credentials file"),
    ("service-account*.json", "a cloud service account key"),
    ("serviceaccount*.json", "a cloud service account key"),
    ("local.settings.json", "Azure Functions settings with connection strings"),
    (".htpasswd", "stored web logins"),
    (".dockercfg", "Docker registry logins"),
    (".pgpass", "stored database passwords"),
    (".vault-token", "a HashiCorp Vault token"),
    (".yarnrc.yml", "Yarn settings that often hold a token"),
    (".bash_history", "a shell history, which often holds passwords"),
    (".zsh_history", "a shell history, which often holds passwords"),
    ("wallet.dat", "a cryptocurrency wallet"),
    (".git", "a Git link file with the developer's local path"),
    ("bach.toml", "the project's own bach.toml, with signing commands and feed addresses"),
    ("bach-*.toml", "a bach.toml of this project, with signing commands and feed addresses"),
];

/// Folder names whose whole contents are secret.
const SECRET_FOLDERS: &[(&str, &str)] = &[
    (".git", "Git history and settings"),
    (".svn", "Subversion history and settings"),
    (".hg", "Mercurial history and settings"),
    (".ssh", "SSH keys"),
    (".aws", "AWS credentials"),
    (".azure", "Azure logins"),
    (".gnupg", "GPG keys"),
    (".docker", "Docker registry logins"),
    (".kube", "Kubernetes cluster logins"),
    (".terraform", "Terraform state and plugins"),
];

/// Paths (matched from the top of the source folder, any depth) whose file
/// name alone is too common to refuse everywhere.
const SECRET_PATHS: &[(&str, &str)] = &[
    ("**/.m2/settings.xml", "Maven settings that often hold passwords"),
    ("**/.m2/settings-security.xml", "Maven master password"),
    ("**/.gradle/gradle.properties", "Gradle settings that often hold credentials"),
    ("**/.gem/credentials", "RubyGems credentials"),
    ("**/.config/gcloud/**", "Google Cloud logins"),
    ("**/.config/gh/hosts.yml", "GitHub CLI logins"),
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
    SECRET_NAMES
        .iter()
        .find(|(pattern, _)| paths::matches(pattern, name))
        .or_else(|| SECRET_PATHS.iter().find(|(pattern, _)| paths::matches(pattern, packaged)))
        .map(|(_, reason)| *reason)
}

/// Files whose text is read for keys and tokens: configuration formats that
/// people paste credentials into. Programs and libraries are never read, since
/// they carry the same words as string constants.
const SCANNED_EXTENSIONS: &[&str] =
    &["json", "yml", "yaml", "toml", "ini", "cfg", "conf", "config", "properties", "txt", "xml", "env", "pem", "ps1", "sh", "bat", "cmd"];
/// Larger files aren't read: a credential file is small.
const SCAN_LIMIT: u64 = 256 * 1024;

/// What a file's text gives away, if it holds a private key or a well-known
/// kind of access token. Only for [`SCANNED_EXTENSIONS`], and only small files.
pub fn content_reason(packaged: &str, path: &std::path::Path, size: u64) -> Option<&'static str> {
    let extension = packaged.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase())?;
    if size > SCAN_LIMIT || !SCANNED_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    if let Some(at) = text.find("-----BEGIN ") {
        let header = &text[at..text.len().min(at + 60)];
        if header.contains("PRIVATE KEY-----") {
            return Some("a private key");
        }
    }
    let has_token = |prefix: &str, length: usize, allowed: &dyn Fn(char) -> bool| {
        text.match_indices(prefix).any(|(at, _)| text[at + prefix.len()..].chars().take_while(|c| allowed(*c)).count() >= length)
    };
    let alphanumeric = |c: char| c.is_ascii_alphanumeric();
    let dashed = |c: char| c.is_ascii_alphanumeric() || c == '-';
    if has_token("AKIA", 16, &|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return Some("an AWS access key");
    }
    if has_token("ghp_", 36, &alphanumeric)
        || has_token("gho_", 36, &alphanumeric)
        || has_token("github_pat_", 40, &|c| alphanumeric(c) || c == '_')
    {
        return Some("a GitHub access token");
    }
    if has_token("xoxb-", 20, &dashed) || has_token("xoxp-", 20, &dashed) {
        return Some("a Slack access token");
    }
    None
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
        .filter_map(|source| {
            secret_reason(&source.packaged)
                .or_else(|| content_reason(&source.packaged, &source.path, source.size))
                .map(|reason| (source.packaged.as_str(), reason))
        })
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
    let clutter: Vec<&str> = sources.iter().map(|source| source.packaged.as_str()).filter(|path| is_clutter(path)).collect();
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
        assert!(
            err.contains("files.source") && err.contains("\".env\"") && err.contains("1 more") && err.contains("files.allow_secrets"),
            "{err}"
        );
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

    #[test]
    fn more_credential_files_are_found() {
        for path in [
            "production.env",
            "app.env",
            "putty.ppk",
            "AuthKey_ABC123.p8",
            "terraform.tfstate",
            "terraform.tfstate.backup",
            "prod.tfvars",
            "credentials.json",
            "service-account-prod.json",
            "local.settings.json",
            ".htpasswd",
            ".dockercfg",
            ".pgpass",
            ".vault-token",
            ".yarnrc.yml",
            ".bash_history",
            "wallet.dat",
            ".git",
            "x/.docker/config.json",
            ".kube/config",
            "a/.azure/accessTokens.json",
            ".svn/entries",
            "sub/.HG/hgrc",
            ".terraform/terraform.tfstate",
            "home/.m2/settings.xml",
            ".gradle/gradle.properties",
            ".gem/credentials",
            ".config/gcloud/credentials.db",
        ] {
            assert!(secret_reason(path).is_some(), "{path} should be a secret");
        }
        for path in ["settings.xml", "m2/settings.xml", "gradle.properties", "docker/config.json", "environment.env.txt", "git.txt"] {
            assert!(secret_reason(path).is_none(), "{path} is fine");
        }
    }

    #[test]
    fn what_a_text_file_holds_gives_it_away() {
        let dir = std::env::temp_dir().join(format!("bach-secrets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            let size = text.len() as u64;
            (path, size)
        };
        // Made of pieces, so no whole fake token sits in the source for a secret scanner to flag.
        let aws = ["AKIA", "IOSFODNN7EXAMPLE"].concat();
        let github = ["gh", "p_", "0123456789abcdefghijklmnopqrstuvwxyz"].concat();
        let slack = ["xo", "xb-", "1234567890-abcdefghijklmn"].concat();
        let cases = [
            ("a.txt", "-----BEGIN OPENSSH PRIVATE KEY-----\nabc", Some("a private key")),
            ("b.json", &format!("{{\"key\":\"{aws}\"}}"), Some("an AWS access key")),
            ("c.yml", &format!("token: {github}"), Some("a GitHub access token")),
            ("d.conf", &format!("slack={slack}"), Some("a Slack access token")),
            ("e.txt", "-----BEGIN CERTIFICATE-----\nabc", None),
            ("f.json", "{\"name\":\"AKIA short\"}", None),
            ("g.txt", "ghp_tooshort", None),
        ];
        for (name, text, expected) in cases {
            let (path, size) = write(name, text);
            assert_eq!(content_reason(name, &path, size), expected, "{name}");
        }
        // A program or a library may carry the same words as constants, and a big file isn't read.
        let (path, size) = write("tool.dll", "-----BEGIN RSA PRIVATE KEY-----");
        assert_eq!(content_reason("tool.dll", &path, size), None);
        let (path, _) = write("big.txt", "-----BEGIN PRIVATE KEY-----");
        assert_eq!(content_reason("big.txt", &path, SCAN_LIMIT + 1), None);
        // `check` refuses by content and `allow_secrets` still lets it through.
        let (path, size) = write("config.json", &format!("{{\"k\":\"{aws}\"}}"));
        let found = vec![Source { packaged: "config.json".into(), path, size }];
        assert!(check(&found, &[], &mut Vec::new()).unwrap_err().to_string().contains("an AWS access key"));
        assert!(check(&found, &["config.json".into()], &mut Vec::new()).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
