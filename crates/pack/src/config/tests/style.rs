//! `installer.style` and `[style]`.

use super::{project_dir, MINIMAL};
use crate::config::parse;
use crate::manifest::{LookFileKind, Theme};
use crate::style::font::tests::font;

fn with(extra: &str) -> String {
    format!("{MINIMAL}\n{extra}")
}

#[test]
fn a_style_is_named_and_brings_its_theme() {
    let dir = project_dir("style-named");
    let project = parse(&with("[installer]\nstyle = \"Arcade\""), &dir).unwrap();
    assert_eq!(project.manifest.look.style.as_deref(), Some("arcade"));
    // Arcade looks best dark, unless the developer picks a theme.
    assert_eq!(project.manifest.look.theme, Theme::Dark);
    let light = parse(&with("[installer]\nstyle = \"arcade\"\ntheme = \"light\""), &dir).unwrap();
    assert_eq!(light.manifest.look.theme, Theme::Light);
    let plain = parse(MINIMAL, &dir).unwrap();
    assert_eq!(plain.manifest.look.style, None);
    assert_eq!(plain.manifest.look.custom, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn colours_apply_to_both_modes_unless_a_mode_has_its_own() {
    let dir = project_dir("style-colours");
    let config = with(
        r##"
        [installer]
        style = "aurora"

        [style]
        accent = "#FF3D81"
        background = ["#1d0b36", "#0a0416"]
        radius = 4

        [style.light]
        background = "#ffffff"
        border = "#00000014"
    "##,
    );
    let project = parse(&config, &dir).unwrap();
    let custom = project.manifest.look.custom.unwrap();
    assert_eq!(custom.radius, Some(4));
    assert_eq!(custom.dark.accent.as_deref(), Some("#ff3d81"));
    assert_eq!(custom.light.accent.as_deref(), Some("#ff3d81"));
    assert_eq!(custom.dark.background_end.as_deref(), Some("#0a0416"));
    // A mode's own background replaces the gradient whole.
    assert_eq!(custom.light.background.as_deref(), Some("#ffffff"));
    assert_eq!(custom.light.background_end, None);
    assert_eq!(custom.light.border.as_deref(), Some("#00000014"));
    assert_eq!(custom.dark.border, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_font_family_or_font_files() {
    let dir = project_dir("style-fonts");
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/Nunito-Regular.ttf"), font(&[(3, 1, 0x0409, 1, "Nunito")], 400)).unwrap();
    std::fs::write(dir.join("fonts/Nunito-Bold.ttf"), font(&[(3, 1, 0x0409, 1, "Nunito Bold"), (3, 1, 0x0409, 16, "Nunito")], 700))
        .unwrap();

    let family = parse(&with("[style]\nfont = \"Segoe UI\""), &dir).unwrap();
    let custom = family.manifest.look.custom.unwrap();
    assert_eq!(custom.font.as_deref(), Some("Segoe UI"));
    // Headings follow the text's font when they have none of their own.
    assert_eq!(custom.heading_font.as_deref(), Some("Segoe UI"));
    assert_eq!(custom.heading_weight, None);
    assert!(family.look_files.is_empty());

    let files = parse(&with("[style]\nfont = [\"fonts/Nunito-Regular.ttf\", \"fonts/Nunito-Bold.ttf\"]"), &dir).unwrap();
    let look = &files.manifest.look;
    let custom = look.custom.as_ref().unwrap();
    assert_eq!(custom.font.as_deref(), Some("Nunito"));
    assert_eq!(custom.heading_weight, Some(700));
    assert_eq!(look.files.iter().map(|file| file.kind).collect::<Vec<_>>(), [LookFileKind::Font, LookFileKind::Font]);
    // The section carries the files after the logo, and splits back into them.
    let parts = look.split_section(files.look_section()).unwrap();
    assert_eq!(parts.fonts, files.look_files);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_background_picture_ships_in_the_setup_file() {
    let dir = project_dir("style-picture");
    std::fs::write(dir.join("backdrop.jpg"), [0xff, 0xd8, 0xff, 0xe0, 1, 2, 3]).unwrap();
    let project = parse(&with("[style]\nbackground_image = \"backdrop.jpg\"\nbackground_image_opacity = 0.5"), &dir).unwrap();
    let look = &project.manifest.look;
    assert_eq!(look.custom.as_ref().unwrap().background_image_opacity, Some(0.5));
    let parts = look.split_section(project.look_section()).unwrap();
    assert_eq!(parts.background, Some((vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3], true)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn style_mistakes_name_their_setting() {
    let dir = project_dir("style-mistakes");
    std::fs::write(dir.join("not-a-font.ttf"), b"hello").unwrap();
    std::fs::write(dir.join("web.woff2"), b"wOF2").unwrap();
    std::fs::write(dir.join("picture.png"), b"GIF89a").unwrap();
    std::fs::write(dir.join("a.ttf"), font(&[(3, 1, 0x0409, 1, "Alpha")], 400)).unwrap();
    std::fs::write(dir.join("b.ttf"), font(&[(3, 1, 0x0409, 1, "Beta")], 400)).unwrap();
    let cases = [
        ("[installer]\nstyle = \"arcde\"", "did you mean \"arcade\""),
        ("[installer]\nstyle = \"vaporwave\"", "use one of bach, aurora"),
        ("[style]\naccent = \"pink\"", "style.accent"),
        ("[style.dark]\ncard = \"#12345\"", "style.dark.card"),
        ("[style]\nbackground = \"#ffffff80\"", "can't be see-through"),
        ("[style]\nbackground = [\"#ffffff\"]", "two for a gradient"),
        ("[style]\nradius = 40", "style.radius"),
        ("[style]\nbutton_radius = 20", "pill buttons"),
        ("[style]\nheading_weight = 50", "style.heading_weight"),
        ("[style]\nfont = \"not-a-font.ttf\"", "isn't a TrueType or OpenType font"),
        ("[style]\nfont = \"web.woff2\"", ".ttf and .otf"),
        ("[style]\nfont = \"missing.ttf\"", "isn't a file"),
        ("[style]\nfont = [\"a.ttf\", \"b.ttf\"]", "two families"),
        ("[style]\nfont = [\"a.ttf\", \"Segoe UI\"]", "only .ttf or .otf"),
        ("[style]\nbackground_image = \"picture.png\"", "isn't a PNG or JPEG"),
        ("[style]\nbackground_image_opacity = 0.5", "style.background_image isn't"),
        ("[installer]\naccent = \"#2563eb\"\n[style]\naccent = \"#ff3d81\"", "both set the accent"),
        ("[style]\nshadow = true", "unknown field `shadow`"),
    ];
    for (extra, expected) in cases {
        let message = format!("{:#}", parse(&with(extra), &dir).unwrap_err());
        assert!(message.contains(expected), "{extra:?} said: {message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
