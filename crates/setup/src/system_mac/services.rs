//! Windows services and firewall rules have no macOS counterpart in a setup
//! file (`bach build` leaves them out). Background programs on a Mac are
//! launch agents, which the app registers itself or asks for through the
//! login item option; so everything here does nothing.

use std::path::Path;

use bach_pack::manifest::Manifest;

/// Why a service or a rule couldn't be set up; never produced on macOS.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub task: String,
    pub code: String,
}

/// The services setup stopped; always none here.
#[derive(Debug, Default)]
pub struct Stopped;

pub fn stop_for_update(_manifest: &Manifest, _previous: Option<&Manifest>, _dir: &Path) -> Result<Stopped, Failure> {
    Ok(Stopped)
}

pub fn restart(_manifest: &Manifest, _stopped: &Stopped, _dir: &Path) {}

pub fn apply(_manifest: &Manifest, _previous: Option<&Manifest>, _dir: &Path, _stopped: &Stopped) -> Result<(), Failure> {
    Ok(())
}

pub fn stop_for_removal(_manifest: &Manifest, _dir: &Path) -> Stopped {
    Stopped
}

pub fn remove(_manifest: &Manifest, _dir: &Path) {}
