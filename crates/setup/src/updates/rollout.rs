//! Staged rollouts: a feed can say `"rollout": 10`, meaning only about one
//! computer in ten should take this version by itself yet. The developer
//! raises the number later (`bach feed --rollout 50`, then leaves it out) so
//! a bad release reaches few people before it is pulled.
//!
//! Which computers are "the first ten percent" must not change from one check
//! to the next, or an update would come and go. Each computer gets a fixed
//! number from 0 to 99, made from its Windows machine id and the app's id, and
//! takes a rollout of `n` percent when that number is below `n`. A rollout
//! of 0 takes nobody; 100, or none, takes everybody. A person who asks for the
//! update themselves (`--update`) always gets it.

use sha2::{Digest, Sha256};
use windows_registry::LOCAL_MACHINE;

/// This computer's number for `app_id`, from 0 to 99.
pub fn this_computer(app_id: &str) -> u8 {
    bucket(&format!("{}|{app_id}", machine_id()))
}

/// A fixed number from 0 to 99 for `seed`.
pub fn bucket(seed: &str) -> u8 {
    let hash = Sha256::digest(seed.as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&hash[..8]);
    (u64::from_le_bytes(first) % 100) as u8
}

/// Whether a computer numbered `bucket` takes a rollout of `percent`.
pub fn included(bucket: u8, percent: Option<u8>) -> bool {
    match percent {
        None => true,
        Some(percent) => bucket < percent.min(100),
    }
}

/// Windows' own id for this installation; the computer's name as a fallback,
/// which is worse (names repeat) but never empty.
fn machine_id() -> String {
    LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Cryptography")
        .ok()
        .and_then(|key| key.get_string("MachineGuid").ok())
        .filter(|id| !id.trim().is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "unknown-computer".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computers_number_never_changes() {
        let first = this_computer("Hello");
        assert!(first < 100);
        assert_eq!(first, this_computer("Hello"));
        assert_eq!(bucket("same seed"), bucket("same seed"));
    }

    #[test]
    fn rollouts_take_the_right_share() {
        assert!(included(0, None) && included(99, None), "no rollout: everybody");
        assert!(!included(0, Some(0)), "0% takes nobody");
        assert!(included(0, Some(1)) && !included(1, Some(1)));
        assert!(included(99, Some(100)) && included(99, Some(250)), "100% and more take everybody");
        assert!(included(49, Some(50)) && !included(50, Some(50)));
    }

    #[test]
    fn numbers_spread_evenly() {
        let mut counts = [0u32; 10];
        for index in 0..20_000 {
            counts[usize::from(bucket(&format!("machine-{index}|Hello"))) / 10] += 1;
        }
        // 2000 each, give or take chance.
        for (tenth, count) in counts.iter().enumerate() {
            assert!((1800..=2200).contains(count), "tenth {tenth} has {count}: {counts:?}");
        }
    }

    #[test]
    fn a_bigger_rollout_keeps_everybody_the_smaller_one_had() {
        for index in 0..2000 {
            let number = bucket(&format!("machine-{index}"));
            if included(number, Some(10)) {
                assert!(included(number, Some(25)) && included(number, Some(100)));
            }
        }
    }

    #[test]
    fn one_app_doesnt_decide_for_another() {
        let same = (0..200).filter(|i| bucket(&format!("m{i}|One")) == bucket(&format!("m{i}|Two"))).count();
        assert!(same < 20, "the same computers shouldn't be first for every app ({same} of 200)");
    }
}
