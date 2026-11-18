//! Opening the setup window: the installer or uninstaller for a payload,
//! or a short explanation when this executable can't install anything. A
//! silent run (`--silent`) never opens a window at all.

use std::sync::Arc;

use bach_pack::i18n::{self, Key};
use bach_pack::manifest::Theme as ThemeChoice;
use gpui::{
    div, px, size, App, AppContext, Application, Bounds, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use crate::assets::{self, Assets};
use crate::launch::{self, Args, Mode};
use crate::package::{Logo, Package};
use crate::pages::{footer, frame, page_icon, row_count, subtitle_text, PageLayout};
use crate::texts::Texts;
use bach_pack::style::layout::{Arrangement, Footer, Rows, SIDE_HEIGHT};

use crate::theme::{ActiveTheme, Looks, Mode as Shade, Pictures};
use crate::ui::FormButton;
use crate::view::{SetupView, Stage};

/// The window's height with up to two rows of choices, in the default
/// layout; each row past the second adds a row's height.
const BASE_HEIGHT: f32 = 490.;
/// The window never grows past this share of the screen; the card scrolls.
const MAX_SCREEN_SHARE: f32 = 0.85;

pub fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(mistake) => {
            // Exit code 2 is "a mistake in the options", as for every silent run.
            crate::system::host::show_command_line_mistake(&mistake);
            std::process::exit(launch::exit::USAGE);
        }
    };
    if args.wants_help() {
        crate::system::host::attach_parent_console();
        print!("{}", launch::HELP);
        return;
    }
    let package = match Package::load() {
        Ok(Some(package)) => Ok(package),
        Ok(None) if args.preview => Ok(Package::sample()),
        Ok(None) => Err(None),
        Err(err) => Err(Some(format!("{err:#}"))),
    };
    // A machine-wide app starts again elevated before it touches anything.
    if let Ok(package) = &package {
        if launch::needs_elevation(&args, package) {
            std::process::exit(launch::elevate_and_wait(&args));
        }
    }
    if let Ok(package) = &package {
        if !args.preview {
            // An update run that starts a setup file must not share its log.
            let path = args.log.clone().unwrap_or_else(|| {
                if args.updating() {
                    crate::log::update_path(&package.manifest.app.id)
                } else {
                    crate::log::default_path(&package.manifest.app.id)
                }
            });
            let app = &package.manifest.app;
            let run: Vec<String> = std::env::args().skip(1).collect();
            crate::log::start(&path, &format!("{} {} setup, Bach {}, run with {run:?}", app.name, app.version, bach_pack::VERSION));
        }
    }
    if args.updating() && !args.preview {
        let code = match &package {
            Ok(package) => crate::updates::run(&args, package),
            Err(_) => 1,
        };
        std::process::exit(code);
    }
    if args.silent && !args.preview {
        let code = match package {
            Ok(package) => crate::silent::run(&args, package),
            Err(_) => {
                crate::system::host::attach_parent_console();
                eprintln!("This setup file is damaged or carries nothing to install.");
                1
            }
        };
        std::process::exit(code);
    }
    match package {
        Ok(package) => {
            let mode = launch::mode(&args, &package);
            let texts = Arc::new(Texts::new(&package.manifest, args.lang.as_deref()));
            let style = bach_pack::style::resolve(&package.manifest.look, args.style.as_deref());
            // A previewed style shows in the theme it looks best in, unless --theme says.
            let preferred = args.style.as_deref().and_then(bach_pack::style::preset).and_then(|preset| preset.theme);
            let theme = args.theme.or(preferred).unwrap_or(package.manifest.look.theme);
            let look = WindowLook {
                style,
                fonts: package.fonts.clone(),
                pictures: Pictures { background: package.picture.clone(), panel: package.panel.clone() },
                layout: package.manifest.look.layout.clone().unwrap_or_default().resolve(),
            };
            let logo_svg = match &package.logo {
                Some(Logo::Svg(bytes)) => Some(bytes.clone()),
                _ => None,
            };
            let removing = matches!(mode, Mode::Remove { .. });
            let title = texts.get(if removing { Key::window_remove } else { Key::window_install });
            let rows = row_count(&package.manifest, removing);
            let tall = package.manifest.locale.license.is_some() && !removing;
            let package = Arc::new(package);
            // The key from the command line, else the one an earlier install
            // of this app kept (so updating by hand doesn't ask again).
            let given_key = args.access_key().ok().flatten().or_else(|| {
                let kept = !args.preview && package.manifest.access.is_some();
                kept.then(|| crate::system::secret::load(&package.manifest.app.id)).flatten()
            });
            open(Assets { logo_svg }, title, theme, look, (rows, tall), move |window, cx| {
                let view = SetupView::new(package, texts, mode, args.preview, given_key, cx);
                window.focus(&view.focus_handle);
                view
            });
        }
        Err(problem) => {
            let theme = args.theme.unwrap_or_default();
            let language = i18n::pick(&[], &args.lang.clone().map(|lang| vec![lang]).unwrap_or_else(crate::system::host::ui_languages));
            let message = match problem {
                // The bare runtime is something only developers see.
                None => "This is the Bach setup runtime on its own. Build an installer with `bach build`, \
                         or look at this window with `bach-setup --preview`."
                    .to_string(),
                Some(error) => i18n::fill(language.strings.problem_damaged, &[("error", &error)]),
            };
            let title = language.strings.problem_title.to_string();
            open(Assets { logo_svg: None }, title.clone(), theme, WindowLook::bach(), (0, false), move |_, _| Problem {
                title: title.into(),
                message: message.into(),
                close: language.strings.close.into(),
            });
        }
    }
}

/// The window's style, and the fonts and picture it ships.
struct WindowLook {
    style: bach_pack::style::Resolved,
    fonts: Vec<Vec<u8>>,
    pictures: Pictures,
    layout: Arrangement,
}

impl WindowLook {
    fn bach() -> Self {
        Self {
            style: bach_pack::style::resolve(&Default::default(), None),
            fonts: Vec::new(),
            pictures: Pictures::default(),
            layout: Arrangement::default_window(),
        }
    }
}

/// Runs the app with one fixed-size window until it closes. `rows` sizes
/// the window for that many rows of choices, or for a license (`tall`).
fn open<V: Render + 'static>(
    assets: Assets,
    title: String,
    theme: ThemeChoice,
    look: WindowLook,
    (rows, tall): (usize, bool),
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
) {
    Application::new().with_assets(assets).run(move |cx: &mut App| {
        if let Err(reason) = assets::load_fonts(cx) {
            eprintln!("bach-setup: Inter didn't load ({reason}); using Segoe UI");
        }
        let (faces, missing) = assets::style_faces(cx, &look.style, &look.fonts);
        if let Some(missing) = missing {
            crate::log::line(&missing);
        }
        let layout = look.layout;
        let looks = Looks::new(&look.style, &faces, look.pictures, layout);
        cx.set_global(looks.get(Shade::Light));
        crate::ui::init(cx);

        let (width, height) = window_size(&layout, rows, tall);
        let screen = cx.primary_display().map(|display| display.bounds().size);
        let (width, height) = match screen {
            Some(screen) if f32::from(screen.height) > 0. => {
                let (screen_width, screen_height) = (f32::from(screen.width), f32::from(screen.height));
                let floor = if layout.height.is_some() { 0. } else { BASE_HEIGHT.min(screen_height) };
                (width.min(screen_width * MAX_SCREEN_SHARE), height.min(screen_height * MAX_SCREEN_SHARE).max(floor))
            }
            _ => (width, height),
        };
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
            is_resizable: false,
            app_id: Some("bach-setup".into()),
            ..Default::default()
        };
        let opened = cx.open_window(options, move |window, cx| {
            follow_appearance(theme, looks, window, cx);
            let view = cx.new(|cx| build(window, cx));
            guard_close(&view, window, cx);
            view
        });
        match opened {
            Ok(_) => cx.activate(true),
            Err(err) => {
                eprintln!("bach-setup couldn't open its window: {err:#}");
                cx.quit();
            }
        }
        // Closing the window ends setup.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}

/// The window's size: `[layout]`'s when it gives one, else tall enough for
/// `rows` rows of choices (or a license, `tall`), as the window always grew.
fn window_size(layout: &Arrangement, rows: usize, tall: bool) -> (f32, f32) {
    if let Some(height) = layout.height {
        return (layout.width, height);
    }
    let row = layout.row_height();
    let [top, _, bottom, _] = layout.padding;
    if layout.side() {
        // The choices and the footer stand alone beside the header.
        let tile_gap = if layout.rows == Rows::Separate { 8. } else { 0. };
        let footer = 16. + layout.button_height + if layout.footer == Footer::Above { 26. } else { 0. };
        let content = top + bottom + layout.gap + rows.max(if tall { 4 } else { 0 }) as f32 * (row + tile_gap) + 2. + footer;
        return (layout.width, content.max(f32::from(SIDE_HEIGHT)));
    }
    let extra = rows.saturating_sub(2) as f32 * row;
    // Padding and type larger than the default make room for themselves.
    let grown = (top + bottom - 68.) + (layout.heading_size - 22.) * 1.5 + (layout.gap - 28.) + (row - 73.) * 2.;
    (layout.width, BASE_HEIGHT + grown.max(-120.) + extra.max(if tall { 2. * row } else { 0. }))
}

/// Keeps the window open while files are moving: closing it then would
/// stop setup halfway, which the all-or-nothing install exists to avoid.
fn guard_close<V: 'static>(view: &gpui::Entity<V>, window: &mut Window, cx: &mut App) {
    let Ok(view) = view.clone().into_any().downcast::<SetupView>() else {
        return;
    };
    let view = view.downgrade();
    window.on_window_should_close(cx, move |_, cx| view.upgrade().is_none_or(|view| !matches!(view.read(cx).stage, Stage::Working { .. })));
}

/// Paints with the chosen theme, or the system's, following it live.
fn follow_appearance(choice: ThemeChoice, looks: Looks, window: &mut Window, cx: &mut App) {
    let apply = move |window: &Window, cx: &mut App| {
        let mode = match choice {
            ThemeChoice::Light => Shade::Light,
            ThemeChoice::Dark => Shade::Dark,
            ThemeChoice::System => Shade::for_appearance(window.appearance()),
        };
        cx.set_global(looks.get(mode));
        cx.refresh_windows();
    };
    apply(window, cx);
    window.observe_window_appearance(move |window, cx| apply(window, cx)).detach();
}

/// Shown when there's nothing to install: a bare runtime, or a damaged file.
struct Problem {
    title: SharedString,
    message: SharedString,
    close: SharedString,
}

impl Render for Problem {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = PageLayout {
            centered: true,
            mark: Some(page_icon("alert-triangle-filled", theme.destructive)),
            subtitle: Some(subtitle_text(self.message.clone(), &theme)),
            ..PageLayout::new(
                self.title.clone(),
                footer(
                    None,
                    vec![FormButton::new("problem-close", self.close.clone()).on_click(|_, _, cx| cx.quit()).into_any_element()],
                    &theme,
                ),
            )
        }
        .render(&theme);
        frame(div().flex_1().min_h_0().flex().flex_col().child(body), &theme)
    }
}
