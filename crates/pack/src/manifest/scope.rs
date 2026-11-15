//! Who an install is for.

use serde::{Deserialize, Serialize};

/// Whether an app installs for the user who runs setup, or for every user of
/// the computer.
///
/// A per-user install needs no administrator rights: it goes into
/// `%LOCALAPPDATA%\Programs` and `HKCU`. A machine-wide install goes into
/// `%ProgramFiles%` and `HKLM`, shows up for every account, and needs
/// administrator rights: setup asks Windows for them once, at the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    User,
    Machine,
}

impl Scope {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "user" => Some(Self::User),
            "machine" => Some(Self::Machine),
            _ => None,
        }
    }

    pub fn is_user(&self) -> bool {
        *self == Self::User
    }

    pub fn is_machine(&self) -> bool {
        *self == Self::Machine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_read_their_names() {
        assert_eq!(Scope::parse(" Machine "), Some(Scope::Machine));
        assert_eq!(Scope::parse("user"), Some(Scope::User));
        assert_eq!(Scope::parse("everyone"), None);
        assert_eq!(serde_json::to_string(&Scope::Machine).unwrap(), "\"machine\"");
        assert!(Scope::default().is_user());
    }
}
