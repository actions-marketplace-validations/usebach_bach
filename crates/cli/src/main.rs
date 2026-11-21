//! `bach`: builds modern, clean Windows installers from a `bach.toml`.
//!
//! ```text
//! bach init              write a starter bach.toml
//! bach check             read bach.toml and list what would be installed
//! bach build             write the setup file
//! bach profiles          list the other builds (portable ZIP, company version) of this bach.toml
//! bach preview           open the installer window without installing anything
//! bach watch             keep the preview open and reopen it when bach.toml changes
//! bach inspect FILE      show what a setup file installs
//! bach test [FILE]       install, start and remove a setup file in Windows Sandbox
//! bach verify FILE       check a setup file for damage without installing it
//! bach diff OLD NEW      show what changed between two setup files of an app
//! bach doctor            check that everything a build needs is in place
//! bach feed FILE         write the update feed for a (signed) setup file
//! bach sign FILE         sign a setup file the way [signing] in bach.toml says
//! bach key               make random access keys for a protected app
//! bach winget FILE --url write winget manifests for a setup file
//! bach choco FILE --url  write the Chocolatey package for a setup file
//! bach page FILE --url   write a download page for a setup file
//! bach intune FILE       write the Intune settings and detection script for a setup file
//! bach version [TARGET]  show or raise the version in every file that has one
//! bach workflow          add the one-click release workflow to a GitHub repository
//! bach update [VERSION]  update bach itself to the newest release (or VERSION)
//! ```

/// `println!` for what a person reads: silent with `--json`, where the command
/// ends with one JSON object instead (see `output::finish`).
macro_rules! say {
    ($($t:tt)*) => {
        if !$crate::output::json() {
            println!($($t)*);
        }
    };
}

mod builder;
mod choco;
mod detect;
mod diff;
mod doctor;
mod feed;
mod github;
mod import;
mod inspect;
mod intune;
mod key;
mod msi;
mod output;
mod page;
mod preview;
mod profiles;
#[cfg(windows)]
mod resources;
mod runtime;
mod sandbox;
mod sign;
mod styles;
mod template;
mod update;
mod verify;
mod version;
mod version_files;
mod watch;
mod winget;
mod workflow;
mod zipfile;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "bach", version, about = "Build modern, clean Windows installers from one bach.toml.")]
struct Cli {
    /// Print one JSON object with the result (and warnings) instead of text lines.
    /// Errors are `{"ok": false, "error": "..."}` with a non-zero exit code.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

/// `bach workflow --pin`.
#[derive(Clone, Copy, ValueEnum)]
enum PinArg {
    Commit,
    Float,
}

/// What `bach preview` and `bach watch` share.
#[derive(clap::Args)]
struct PreviewArgs {
    /// The config file.
    #[arg(default_value = "bach.toml")]
    config: PathBuf,
    /// Show the uninstaller instead.
    #[arg(long)]
    uninstall: bool,
    /// Override the theme: system, light or dark.
    #[arg(long)]
    theme: Option<String>,
    /// Try another built-in style without changing bach.toml (`bach styles` lists them).
    #[arg(long)]
    style: Option<String>,
    /// Try another built-in layout without changing bach.toml (`bach layouts` lists them).
    #[arg(long)]
    layout: Option<String>,
    /// Show it in this language (en, de, fr, …) instead of Windows' own.
    #[arg(long)]
    lang: Option<String>,
    /// The setup runtime to build on (default: bach-setup.exe next to bach).
    #[arg(long, env = "BACH_RUNTIME")]
    runtime: Option<PathBuf>,
    /// Show this profile of the config (`bach profiles` lists them).
    #[arg(long)]
    profile: Option<String>,
}

impl PreviewArgs {
    fn options(&self) -> preview::PreviewOptions<'_> {
        preview::PreviewOptions {
            uninstall: self.uninstall,
            theme: self.theme.as_deref(),
            style: self.style.as_deref(),
            layout: self.layout.as_deref(),
            lang: self.lang.as_deref(),
            runtime: self.runtime.as_deref(),
            profile: self.profile.as_deref(),
        }
    }
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
        /// Check this profile of the config (`bach profiles` lists them).
        #[arg(long)]
        profile: Option<String>,
        /// Fail when there is a warning, so a pipeline can't ship what check
        /// complains about (or set BACH_STRICT=1).
        #[arg(long, env = "BACH_STRICT")]
        strict: bool,
    },
    /// Build the setup file.
    Build {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
        /// Build this profile instead of the main setup file; repeat it for
        /// several (`bach profiles` lists them).
        #[arg(long)]
        profile: Vec<String>,
        /// Build the main setup file and every profile.
        #[arg(long, conflicts_with = "profile")]
        all_profiles: bool,
        /// The ARM64 setup runtime (bach-setup.exe from the ARM64 release) for
        /// [arch.arm64] in bach.toml, when the config doesn't name one.
        #[arg(long, env = "BACH_RUNTIME_ARM64")]
        runtime_arm64: Option<PathBuf>,
        /// Build only the x64 setup file, even when bach.toml has [arch.arm64].
        #[arg(long)]
        x64_only: bool,
        /// Also write a Windows Installer package (.msi) of the app, for
        /// Group Policy, Configuration Manager and Intune.
        #[arg(long)]
        msi: bool,
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
        /// Stop before building when bach.toml has a warning (or set
        /// BACH_STRICT=1), like `bach check --strict`.
        #[arg(long, env = "BACH_STRICT")]
        strict: bool,
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
    /// List the profiles of a bach.toml: other builds of the app, like a
    /// portable ZIP or a version for companies.
    Profiles {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
    },
    /// Open the installer window as it will look, without installing anything.
    Preview(PreviewArgs),
    /// Keep the preview open, and reopen it whenever bach.toml (or a file it
    /// names, like the logo or the license) changes.
    Watch(PreviewArgs),
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
        /// The ARM64 setup file of the same version (built with the ARM64
        /// runtime), to list for computers that run on ARM64.
        #[arg(long)]
        arm64: Option<PathBuf>,
        /// The https link the ARM64 setup file is downloaded from (default:
        /// the download link of [updates] in bach.toml, for its file name).
        #[arg(long, requires = "arm64")]
        arm64_url: Option<String>,
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
    /// Write the Chocolatey package that installs a setup file.
    Choco {
        /// The setup file.
        file: PathBuf,
        /// The https link it's downloaded from.
        #[arg(long)]
        url: String,
        /// The app's license: a link, or an SPDX name like MIT.
        #[arg(long)]
        license: Option<String>,
        /// Write the package into this folder.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Write a first bach.toml from an Inno Setup script (.iss) or an NSIS script
    /// (.nsi): the app's name, files, shortcuts, license and steps. What has no
    /// Bach setting is listed, not guessed.
    Import {
        /// The script to read.
        script: PathBuf,
        /// Replace a bach.toml that is already next to the script.
        #[arg(long)]
        force: bool,
        /// Print the bach.toml instead of writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Try a setup file on a clean Windows in Windows Sandbox: install it
    /// silently, start the app, remove it, and report what failed and what
    /// was left behind.
    Test {
        /// The setup file (default: the one bach.toml builds).
        file: Option<PathBuf>,
        /// The config file, to find the setup file.
        #[arg(long, default_value = "bach.toml")]
        config: PathBuf,
        /// Install with these options on (ids as for the setup file's --with).
        #[arg(long, value_delimiter = ',')]
        with: Vec<String>,
        /// Install with these options off.
        #[arg(long, value_delimiter = ',')]
        without: Vec<String>,
        /// How many seconds to leave the app running after it starts; 0 doesn't start it.
        #[arg(long, default_value_t = 5)]
        launch: u32,
        /// Give the sandbox the network (off by default, so the test needs nothing from outside).
        #[arg(long)]
        network: bool,
        /// Give up waiting for the sandbox after this many minutes.
        #[arg(long, default_value_t = 20)]
        timeout: u32,
        /// Leave the sandbox open when the test ends, to look around in it.
        #[arg(long)]
        keep: bool,
        /// Write the files the test would use and stop (works without Windows Sandbox).
        #[arg(long)]
        dry_run: bool,
        /// Run the test on this computer instead of in Windows Sandbox: it installs the app
        /// for real and removes it again. For a CI runner or a virtual machine; it refuses on a
        /// computer that already has the app.
        #[arg(long, conflicts_with = "keep")]
        here: bool,
    },
    /// Show what changed between two setup files of one app: the files added,
    /// removed and changed, what the new one writes to the registry and the
    /// environment, and which files made the installer bigger.
    Diff {
        /// The older setup file.
        old: PathBuf,
        /// The newer setup file.
        new: PathBuf,
        /// How many of the files that grew the most to list.
        #[arg(long, default_value_t = 5)]
        top: usize,
    },
    /// Check a setup file for damage, without installing it: the archive and
    /// every file against the checksums recorded when it was built.
    Verify {
        /// The setup file.
        file: PathBuf,
        /// The access key's file, to check the files of an encrypted setup
        /// file too (or set BACH_ACCESS_KEY).
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Write a download page for a setup file: one HTML file with the name,
    /// version, size, a download button and the SHA-256.
    Page {
        /// The setup file.
        file: PathBuf,
        /// The https link the setup file is downloaded from.
        #[arg(long)]
        url: String,
        /// Write the page into this folder (default: download/ next to the setup file).
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// A file with what's new (shown as a list; Markdown is not interpreted).
        #[arg(long)]
        notes_file: Option<PathBuf>,
        /// The app's winget package id, to show `winget install ID`.
        #[arg(long)]
        winget: Option<String>,
    },
    /// Write what Microsoft Intune needs to deploy a setup file: the install
    /// and uninstall commands and a detection script.
    Intune {
        /// The setup file.
        file: PathBuf,
        /// Write the files into this folder.
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
        /// How the workflow names Bach's action: `commit` (default) pins the
        /// commit of this release, `float` follows the newest 0.x release.
        #[arg(long, value_enum, default_value = "commit")]
        pin: PinArg,
        /// The app's winget package id (like Example.Hello): also open the
        /// pull request that updates it in winget-pkgs after each release.
        #[arg(long, value_name = "ID")]
        winget: Option<String>,
    },
    /// List the built-in styles of the installer window, for installer.style.
    Styles,
    /// List the built-in layouts of the installer window, for [layout] preset.
    Layouts,
    /// Check that everything a build needs is in place.
    Doctor {
        /// The config file.
        #[arg(default_value = "bach.toml")]
        config: PathBuf,
    },
    /// Update bach itself to the newest release on GitHub.
    Update {
        /// Install exactly this version, like 0.8.0 (an older one is fine).
        version: Option<String>,
        /// Only say whether a newer release exists; change nothing.
        #[arg(long, conflicts_with_all = ["version", "force"])]
        check: bool,
        /// Install even if this version is already the one you have.
        #[arg(long)]
        force: bool,
    },
    /// Looks up the newest release for the update notice (started by bach itself).
    #[command(name = "update-refresh", hide = true)]
    UpdateRefresh,
}

fn main() -> ExitCode {
    update::tidy();
    let cli = Cli::parse();
    output::set_json(cli.json);
    // `bach update` says what there is to say itself, and the background
    // check must stay silent.
    let notify = !matches!(cli.command, Command::Update { .. } | Command::UpdateRefresh);
    let result = match cli.command {
        Command::Update { version, check, force } => update::command(update::Options { version: version.as_deref(), check, force }),
        Command::UpdateRefresh => update::refresh(),
        Command::Init { dir, no_workflow } => template::init(&dir, !no_workflow),
        Command::Check { config, app_version, profile, strict } => {
            builder::check(&config, app_version.as_deref(), profile.as_deref(), strict)
        }
        Command::Profiles { config } => profiles::command(&config),
        Command::Build {
            config,
            profile,
            all_profiles,
            runtime_arm64,
            x64_only,
            msi,
            out,
            runtime,
            fast,
            app_version,
            online,
            key_file,
            sign,
            patch_from,
            strict,
        } => builder::build_all(
            builder::BuildOptions {
                config: &config,
                runtime_arm64: runtime_arm64.as_deref(),
                x64_only,
                msi,
                arch: None,
                profile: None,
                out: out.as_deref(),
                runtime: runtime.as_deref(),
                fast,
                version: app_version.as_deref(),
                online,
                key_file: key_file.as_deref(),
                sign,
                patch_from: patch_from.as_deref(),
                strict,
            },
            &profile,
            all_profiles,
        ),
        Command::Key { count, out } => key::command(count, out.as_deref()),
        Command::FeedKey { out } => feed::key_command(out.as_deref()),
        Command::Sign { file, config } => sign::command(sign::Options { file: &file, config: &config }),
        Command::Preview(args) => preview::preview(&args.config, args.options()),
        Command::Watch(args) => watch::command(watch::Options { config: &args.config, preview: args.options() }),
        Command::Inspect { file } => inspect::inspect(&file),
        Command::Styles => styles::command(),
        Command::Layouts => styles::layouts(),
        Command::Doctor { config } => doctor::doctor(&config),
        Command::Version { target, config, dry_run } => {
            version::command(version::Options { config: &config, target: target.as_deref(), dry_run })
        }
        Command::Workflow { dir, force, pin, winget } => workflow::command(workflow::Options {
            dir: &dir,
            force,
            winget: winget.as_deref(),
            pin: match pin {
                PinArg::Commit => workflow::Pin::Commit,
                PinArg::Float => workflow::Pin::Float,
            },
        }),
        Command::Winget { file, url, license, out } => {
            winget::command(winget::Options { setup: &file, url: &url, license: license.as_deref(), out: out.as_deref() })
        }
        Command::Choco { file, url, license, out } => {
            choco::command(choco::Options { setup: &file, url: &url, license: license.as_deref(), out: out.as_deref() })
        }
        Command::Test { file, config, with, without, launch, network, timeout, keep, dry_run, here } => {
            sandbox::command(sandbox::Options {
                file: file.as_deref(),
                config: &config,
                with: &with,
                without: &without,
                launch_seconds: launch,
                network,
                timeout_minutes: timeout,
                keep,
                dry_run,
                here,
            })
        }
        Command::Import { script, force, dry_run } => import::command(import::Options { script: &script, force, dry_run }),
        Command::Diff { old, new, top } => diff::command(diff::Options { old: &old, new: &new, top }),
        Command::Verify { file, key_file } => verify::command(verify::Options { file: &file, key_file: key_file.as_deref() }),
        Command::Page { file, url, out, notes_file, winget } => page::command(page::Options {
            setup: &file,
            url: &url,
            out: out.as_deref(),
            notes_file: notes_file.as_deref(),
            winget: winget.as_deref(),
        }),
        Command::Intune { file, out } => intune::command(intune::Options { setup: &file, out: out.as_deref() }),
        Command::Feed { file, url, config, notes, notes_file, rollout, out, arm64, arm64_url, patch, patch_url } => {
            feed::command(feed::Options {
                arm64: arm64.as_deref(),
                arm64_url: arm64_url.as_deref(),
                setup: &file,
                url: url.as_deref(),
                config: config.as_deref(),
                notes: notes.as_deref(),
                notes_file: notes_file.as_deref(),
                rollout,
                out: out.as_deref(),
                patch: patch.as_deref(),
                patch_url: patch_url.as_deref(),
            })
        }
    };
    let code = match result {
        Ok(()) => {
            output::finish(None);
            ExitCode::SUCCESS
        }
        Err(err) => {
            output::error(&err);
            output::finish(Some(&err));
            ExitCode::FAILURE
        }
    };
    if notify {
        update::notify();
    }
    code
}
