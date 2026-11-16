//! Which update channel this installation follows: the stable one (what
//! `[updates] feed` serves) or the beta one (`[updates] beta_feed`).
//!
//! It is a choice of the person using the app, so it lives under their own
//! `HKCU\Software\Bach\Channel`, one value per app, and an update, a repair
//! or a reinstall leaves it as it is. Removing the app deletes it.

use anyhow::{Context, Result};
use windows_registry::CURRENT_USER;

const KEY: &str = r"Software\Bach\Channel";

/// Whether `app_id` follows the beta channel. No value, or anything but
/// `beta`, is the stable channel.
pub fn is_beta(app_id: &str) -> bool {
    CURRENT_USER.open(KEY).ok().and_then(|key| key.get_string(app_id).ok()).is_some_and(|channel| channel.eq_ignore_ascii_case("beta"))
}

/// Follows the beta channel, or goes back to stable.
pub fn set(app_id: &str, beta: bool) -> Result<()> {
    if beta {
        CURRENT_USER
            .create(KEY)
            .context("couldn't open the place the update channel is kept")?
            .set_string(app_id, "beta")
            .context("couldn't keep the update channel")?;
    } else {
        forget(app_id);
    }
    Ok(())
}

/// Deletes the choice kept for `app_id`; one that isn't there is already gone.
pub fn forget(app_id: &str) {
    if let Ok(key) = CURRENT_USER.options().read().write().open(KEY) {
        let _ = key.remove_value(app_id);
        let empty = key.keys().is_ok_and(|mut keys| keys.next().is_none()) && key.values().is_ok_and(|mut values| values.next().is_none());
        drop(key);
        if empty {
            let _ = CURRENT_USER.remove_tree(KEY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_kept_per_app_and_goes_with_the_app() {
        let (one, two) = (format!("BachQA.channel1.{}", std::process::id()), format!("BachQA.channel2.{}", std::process::id()));
        assert!(!is_beta(&one), "nothing chosen is stable");
        set(&one, true).unwrap();
        assert!(is_beta(&one) && !is_beta(&two), "one app's choice isn't another's");
        set(&one, true).unwrap();
        assert!(is_beta(&one));
        set(&one, false).unwrap();
        assert!(!is_beta(&one), "back to stable");
        set(&two, true).unwrap();
        forget(&two);
        assert!(!is_beta(&two));
    }
}
