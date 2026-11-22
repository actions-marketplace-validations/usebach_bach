//! The payload this executable carries: read once at startup from its own
//! file, or a built-in sample for `--preview` on a bare runtime.

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bach_pack::container::{self, Layout};
use bach_pack::manifest::{self, FileEntry, LogoKind, Manifest};

/// Everything setup knows about the app it installs or removes.
pub struct Package {
    pub manifest: Manifest,
    /// The file the payload was read from: this executable on Windows, and on a
    /// Mac the `setup.bach` beside it in the app bundle.
    pub exe: PathBuf,
    /// The program that is running: this executable. Copied to make the uninstaller.
    #[cfg_attr(windows, allow(dead_code))]
    pub runtime: PathBuf,
    /// None for the built-in sample, which has nothing to unpack.
    pub layout: Option<Layout>,
    pub logo: Option<Logo>,
    /// Font files the style ships, registered before the window opens.
    pub fonts: Vec<Vec<u8>>,
    /// The style's background picture.
    pub picture: Option<Arc<gpui::Image>>,
    /// The picture in a side header's panel (`[layout] panel_image`).
    pub panel: Option<Arc<gpui::Image>>,
}

pub enum Logo {
    /// Served to GPUI as an asset and drawn in the theme's ink.
    Svg(Arc<[u8]>),
    /// Drawn as is.
    Png(Arc<gpui::Image>),
}

impl Package {
    /// The payload of the running executable; Ok(None) when it's a bare
    /// runtime.
    pub fn load() -> Result<Option<Self>> {
        let exe = std::env::current_exe().context("couldn't find this setup file")?;
        Self::load_from(exe)
    }

    /// Where the payload is for a program at `exe`: the program itself, or on a
    /// Mac `Contents/Resources/setup.bach` of the bundle it runs from. A program
    /// outside a bundle (`cargo run`) has none.
    #[cfg(target_os = "macos")]
    fn payload_path(exe: &std::path::Path) -> PathBuf {
        bach_pack::macapp::PAYLOAD
            .strip_prefix("Contents/")
            .and_then(|inside| exe.parent()?.parent().map(|contents| contents.join(inside)))
            .unwrap_or_else(|| exe.to_path_buf())
    }

    #[cfg(windows)]
    fn payload_path(exe: &std::path::Path) -> PathBuf {
        exe.to_path_buf()
    }

    /// The payload of the setup file at `exe`: this executable, or the full
    /// setup file an online installer downloaded.
    pub fn load_from(exe: PathBuf) -> Result<Option<Self>> {
        let payload = Self::payload_path(&exe);
        let mut file = match File::open(&payload) {
            Ok(file) => file,
            // A bare runtime on a Mac has no payload file at all.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && cfg!(target_os = "macos") => return Ok(None),
            Err(err) => return Err(err).context("couldn't open this setup file"),
        };
        let Some(layout) = container::locate(&mut file)? else {
            return Ok(None);
        };
        let manifest = container::read_manifest(&mut file, &layout)?;
        let look = container::read_look(&mut file, &layout, &manifest)?;
        let bytes = look.logo;
        let image = |(bytes, jpeg): (Vec<u8>, bool)| {
            let format = if jpeg { gpui::ImageFormat::Jpeg } else { gpui::ImageFormat::Png };
            Arc::new(gpui::Image::from_bytes(format, bytes))
        };
        let picture = look.background.map(image);
        let panel = look.panel.map(image);
        let logo = match manifest.look.logo {
            _ if bytes.is_empty() => None,
            Some(LogoKind::Svg) => Some(Logo::Svg(bytes.into())),
            Some(LogoKind::Png) => Some(Logo::Png(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)))),
            None => None,
        };
        Ok(Some(Self { manifest, exe: payload, runtime: exe, layout: Some(layout), logo, fonts: look.fonts, picture, panel }))
    }

    /// A made-up app for previewing the window on a bare runtime.
    pub fn sample() -> Self {
        let mac = cfg!(target_os = "macos");
        let manifest = Manifest {
            format: bach_pack::FORMAT_VERSION,
            builder: bach_pack::VERSION.into(),
            agents: Vec::new(),
            platform: if mac { manifest::Platform::Macos } else { manifest::Platform::Windows },
            mac: mac.then(|| manifest::MacInfo { bundle_id: "app.bach.setup.sample".into(), minimum_version: None }),
            app: manifest::App {
                id: "Sample".into(),
                name: "Sample".into(),
                version: "1.0.0".into(),
                publisher: "Sample Inc.".into(),
                description: Some("A preview of the installer window.".into()),
                website: None,
                executable: if mac { "Sample.app".into() } else { "sample.exe".into() },
            },
            install: manifest::Install {
                folder: "Sample".into(),
                desktop_shortcut: manifest::Choice::On,
                start_menu_shortcut: manifest::Choice::Always,
                start_with_windows: manifest::Choice::Never,
                add_to_path: manifest::Choice::Never,
                path_folder: None,
                change_folder: true,
                open_button: true,
                start_menu_folder: None,
                website_button: false,
                scope: manifest::Scope::User,
            },
            uninstall: manifest::Uninstall {
                data: vec![if mac { "~/Library/Application Support/Sample".into() } else { r"%APPDATA%\Sample".into() }],
                data_description: None,
                feedback_url: None,
            },
            look: manifest::Look::default(),
            files: vec![FileEntry::new(if mac { "Sample.app/Contents/MacOS/sample" } else { "sample.exe" }, 48 << 20)],
            components: Vec::new(),
            protocols: Vec::new(),
            file_types: Vec::new(),
            locale: manifest::Locale::default(),
            chosen: None,
            requirements: Vec::new(),
            shortcuts: Vec::new(),
            run: Vec::new(),
            system: manifest::SystemChanges::default(),
            updates: None,
            access: None,
            online: false,
            archive_sha256: None,
            patch_from: None,
        };
        Self {
            manifest,
            exe: PathBuf::new(),
            runtime: PathBuf::new(),
            layout: None,
            logo: None,
            fonts: Vec::new(),
            picture: None,
            panel: None,
        }
    }

    /// Whether this carries files to install.
    pub fn has_files(&self) -> bool {
        self.layout.is_some_and(|layout| layout.has_archive())
    }

    /// Whether this is an installer rather than an uninstaller: one that
    /// carries its files, or an online one that downloads them.
    pub fn is_installer(&self) -> bool {
        self.has_files() || self.manifest.online
    }

    pub fn version(&self) -> &str {
        &self.manifest.app.version
    }
}
