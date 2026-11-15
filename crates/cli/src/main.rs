//! `bach`: builds modern, clean Windows installers from a `bach.toml`.
//!
//! ```text
//! bach init              write a starter bach.toml
//! bach check             read bach.toml and list what would be installed
//! bach build             write the setup file
//! bach preview           open the installer window without installing anything
//! bach inspect FILE      show what a setup file installs
//! bach doctor            check that everything a build needs is in place
//! bach feed FILE         write the update feed for a (signed) setup file
//! bach sign FILE         sign a setup file the way [signing] in bach.toml says
//! bach key               make random access keys for a protected app
//! bach winget FILE --url write winget manifests for a setup file
//! bach version [TARGET]  show or raise the version in every file that has one
//! bach workflow          add the one-click release workflow to a GitHub repository
//! ```

mod builder;
mod detect;
mod doctor;
mod feed;
mod github;
mod inspect;
mod key;
mod output;
#[cfg(windows)]
mod resources;
mod runtime;
mod sign;
mod template;
mod version;
mod version_files;
mod winget;
mod workflow;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bach", version, about = "Build modern, clean Windows installers from one bach.toml.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter bach.toml in this folder (or the one given).
    Init {
        /// Where to write it.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Don't add the GitHub release workflow, even in a GitHub checkout.
        #[arg(long)]
        no_workflow: bool,
    },
    /// Read a bach.toml and list what the installer would install.
    Check {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
        /// Use this version instead of app.version (a leading v is dropped).
        #[arg(long)]
        app_version: Option<String>,
    },
    /// Build the setup file.
    Build {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
        /// Write the setup file here instead of installer.output.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// The setup runtime to build on (default: bach-setup.exe next to bach).
        #[arg(long, env = "BACH_RUNTIME")]
        runtime: Option<PathBuf>,
        /// Compress quickly instead of as small as possible.
        #[arg(long)]
        fast: bool,
        /// Use this version instead of app.version, so a release tag like
        /// v1.4.0 is the only place the version is written (a leading v is dropped).
        #[arg(long)]
        app_version: Option<String>,
        /// Also write the online installer, which downloads the newest
        /// version when it runs (same as installer.online = true).
        #[arg(long)]
        online: bool,
        /// Read the access key from this file instead of the environment
        /// variable [access] names.
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Sign the setup files with the program [signing] in bach.toml names
        /// (or set BACH_SIGN=1). Off by default: a bach.toml never runs a
        /// program unless you say so.
        #[arg(long, env = "BACH_SIGN")]
        sign: bool,
        /// An earlier setup file of this app. Also writes an update patch
        /// from it that holds only the files that changed, and offers it in
        /// the update feed to computers that have that version.
        #[arg(long)]
        patch_from: Option<PathBuf>,
    },
    /// Sign a setup file with the program [signing] in bach.toml names.
    Sign {
        /// The setup file.
        file: PathBuf,
        /// The config file.
        #[arg(long, default_value = "bach.toml")]
        config: PathBuf,
    },
    /// Make random access keys for an app with [access].
    Key {
        /// How many keys to make (one per customer, say).
        #[arg(long, short, default_value_t = 1)]
        count: u32,
        /// Write the keys to this new file (UTF-8, one per line) instead of
        /// printing them; `bach build --key-file FILE` reads it.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Open the installer window as it will look, without installing anything.
    Preview {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
        /// Show the uninstaller instead.
        #[arg(long)]
        uninstall: bool,
        /// Override the theme: system, light or dark.
        #[arg(long)]
        theme: Option<String>,
        /// Show it in this language (en, de, fr, …) instead of Windows' own.
        #[arg(long)]
        lang: Option<String>,
        /// The setup runtime to build on (default: bach-setup.exe next to bach).
        #[arg(long, env = "BACH_RUNTIME")]
        runtime: Option<PathBuf>,
    },
    /// Show what a setup file installs.
    Inspect {
        /// A setup file (or an installed uninstall.exe).
        file: PathBuf,
    },
    /// Make the key pair that signs your update feeds (see [updates] public_key).
    FeedKey {
        /// Where to write the private key (default: feed-key.private; it must not exist).
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Write the update feed (latest.json) for a setup file, after signing it.
    Feed {
        /// The setup file.
        file: PathBuf,
        /// The https link it will be downloaded from (default: the download
        /// link of [updates] in bach.toml).
        #[arg(long)]
        url: Option<String>,
        /// The bach.toml to take the download link from (default: bach.toml here).
        #[arg(long)]
        config: Option<PathBuf>,
        /// A line about what's new.
        #[arg(long)]
        notes: Option<String>,
        /// A file with what's new (Markdown is fine).
        #[arg(long, conflicts_with = "notes")]
        notes_file: Option<PathBuf>,
        /// Roll out to this percentage of computers first (0 to 100); installed
        /// apps outside it wait until you write the feed again without it.
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        rollout: Option<u8>,
        /// Write the feed here instead of latest.json next to the setup file.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// An update patch made by `bach build --patch-from`, to offer to
        /// computers that have the version it updates from.
        #[arg(long)]
        patch: Option<PathBuf>,
        /// The https link the patch is downloaded from (default: the download
        /// link of [updates] in bach.toml, for the patch's file name).
        #[arg(long, requires = "patch")]
        patch_url: Option<String>,
    },
    /// Write the manifests that list a setup file in winget.
    Winget {
        /// The setup file.
        file: PathBuf,
        /// The https link it's downloaded from.
        #[arg(long)]
        url: String,
        /// The app's license, like MIT (default: Proprietary).
        #[arg(long)]
        license: Option<String>,
        /// Write the manifests into this folder.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Show the version a project carries, or raise it in every file at once.
    Version {
        /// patch, minor, major, current (keep it), or an exact version like
        /// 2.0.0. Leave it out to list the versions instead.
        target: Option<String>,
        /// The config file; the version files are the ones next to it.
        #[arg(long, default_value = "bach.toml")]
        config: PathBuf,
        /// Say what would change, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add the one-click release workflow to this GitHub repository.
    Workflow {
        /// The folder with the bach.toml.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Replace .github/workflows/bach-release.yml even if Bach didn't write it.
        #[arg(long)]
        force: bool,
    },
    /// Check that everything a build needs is in place.
    Doctor {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Init { dir, no_workflow } => template::init(&dir, !no_workflow),
        Command::Check { config, app_version } => builder::check(&config, app_version.as_deref()),
        Command::Build { config, out, runtime, fast, app_version, online, key_file, sign, patch_from } => {
            builder::build(builder::BuildOptions {
                config: &config,
                out: out.as_deref(),
                runtime: runtime.as_deref(),
                fast,
                version: app_version.as_deref(),
                online,
                key_file: key_file.as_deref(),
                sign,
                patch_from: patch_from.as_deref(),
            })
            .map(|_| ())
        }
        Command::Key { count, out } => key::command(count, out.as_deref()),
        Command::FeedKey { out } => feed::key_command(out.as_deref()),
        Command::Sign { file, config } => sign::command(sign::Options { file: &file, config: &config }),
        Command::Preview { config, uninstall, theme, lang, runtime } => builder::preview(
            &config,
            builder::PreviewOptions { uninstall, theme: theme.as_deref(), lang: lang.as_deref(), runtime: runtime.as_deref() },
        ),
        Command::Inspect { file } => inspect::inspect(&file),
        Command::Doctor { config } => doctor::doctor(&config),
        Command::Version { target, config, dry_run } => {
            version::command(version::Options { config: &config, target: target.as_deref(), dry_run })
        }
        Command::Workflow { dir, force } => workflow::command(workflow::Options { dir: &dir, force }),
        Command::Winget { file, url, license, out } => {
            winget::command(winget::Options { setup: &file, url: &url, license: license.as_deref(), out: out.as_deref() })
        }
        Command::Feed { file, url, config, notes, notes_file, rollout, out, patch, patch_url } => feed::command(feed::Options {
            setup: &file,
            url: url.as_deref(),
            config: config.as_deref(),
            notes: notes.as_deref(),
            notes_file: notes_file.as_deref(),
            rollout,
            out: out.as_deref(),
            patch: patch.as_deref(),
            patch_url: patch_url.as_deref(),
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            output::error(&err);
            ExitCode::FAILURE
        }
    }
}
