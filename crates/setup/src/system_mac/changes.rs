//! Registry values, environment variables and right-click entries are Windows
//! ideas: a macOS setup file has none of them (`bach build` leaves them out), so
//! applying and undoing them does nothing.

use std::path::Path;

use anyhow::Result;
use bach_pack::manifest::Manifest;

/// Where the changes would go on Windows; kept so the install code reads the same.
pub struct Where<'a>(std::marker::PhantomData<&'a ()>);

pub fn at(_manifest: &Manifest) -> Where<'static> {
    Where(std::marker::PhantomData)
}

pub fn apply(_at: &Where, _manifest: &Manifest, _previous: Option<&Manifest>, _dir: &Path, _exe: &Path) -> Result<()> {
    Ok(())
}

pub fn undo(_at: &Where, _manifest: &Manifest, _dir: &Path) {}
