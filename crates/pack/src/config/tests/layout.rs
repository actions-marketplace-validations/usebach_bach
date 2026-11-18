//! `[layout]`.

use super::{project_dir, MINIMAL};
use crate::config::parse;
use crate::manifest::LookFileKind;
use crate::style::layout::{Align, Animation, Buttons, Footer, HeaderPlace, Panel, Rows, Surface};

fn with(extra: &str) -> String {
    format!("{MINIMAL}\n{extra}")
}

#[test]
fn a_layout_is_read_as_written() {
    let dir = project_dir("layout-read");
    std::fs::write(dir.join("side.jpg"), [0xff, 0xd8, 0xff, 0xe0, 9]).unwrap();
    let config = with(
        r#"
        [layout]
        width = 820
        height = 480
        panel = "Left"
        panel_size = 300
        panel_inset = 12
        panel_radius = 20
        panel_image = "side.jpg"
        header = "panel"
        content_width = 420
        surface = "card"
        align = "left"
        padding = [40, 28]
        heading_size = 28
        text_size = 15
        rows = "separate"
        columns = 2
        icons = false
        buttons = "stretch"
        footer = "hidden"
        animation = "slide"
        animation_speed = 1.5
    "#,
    );
    let project = parse(&config, &dir).unwrap();
    let look = &project.manifest.look;
    let layout = look.layout.as_ref().unwrap();
    assert_eq!((layout.width, layout.height, layout.panel), (Some(820), Some(480), Some(Panel::Left)));
    assert_eq!((layout.panel_size, layout.panel_inset, layout.panel_radius), (Some(300), Some(12), Some(20)));
    assert_eq!((layout.header, layout.content_width, layout.surface), (Some(HeaderPlace::Panel), Some(420), Some(Surface::Card)));
    assert_eq!(layout.padding, Some([40, 28, 40, 28]));
    assert_eq!((layout.align, layout.rows, layout.columns, layout.icons), (Some(Align::Left), Some(Rows::Separate), Some(2), Some(false)));
    assert_eq!((layout.buttons, layout.footer, layout.animation), (Some(Buttons::Stretch), Some(Footer::Hidden), Some(Animation::Slide)));
    assert_eq!(layout.animation_speed, Some(1.5));
    let parts = look.split_section(project.look_section()).unwrap();
    assert_eq!(parts.panel, Some((vec![0xff, 0xd8, 0xff, 0xe0, 9], true)));
    assert_eq!(look.files.last().map(|file| file.kind), Some(LookFileKind::PanelJpeg));
    // An empty [layout] changes nothing.
    assert_eq!(parse(&with("[layout]"), &dir).unwrap().manifest.look.layout, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_preset_is_a_starting_point() {
    let dir = project_dir("layout-preset");
    let project = parse(&with("[layout]\npreset = \"Showcase\"\nheading_size = 36\npanel = \"left\""), &dir).unwrap();
    let layout = project.manifest.look.layout.unwrap();
    assert_eq!(layout.preset.as_deref(), Some("showcase"));
    assert_eq!((layout.heading_size, layout.panel, layout.panel_radius), (Some(36), Some(Panel::Left), Some(26)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn layout_mistakes_name_their_setting() {
    let dir = project_dir("layout-mistakes");
    let cases = [
        ("preset = \"showcas\"", "did you mean \"showcase\""),
        ("preset = \"carousel\"", "use one of classic, showcase"),
        ("width = 100", "layout.width is 100; use 380 to 1400"),
        ("height = 5000", "layout.height"),
        ("panel = \"bottom\"", "use \"none\", \"left\", \"right\" or \"top\""),
        ("header = \"panel\"", "there is no panel"),
        ("panel_image = \"x.png\"", "there is no panel"),
        ("panel = \"left\"\npanel_radius = 90", "layout.panel_radius"),
        ("align = \"middle\"", "layout.align"),
        ("padding = [1, 2, 3]", "one number (every side), two"),
        ("padding = 200", "layout.padding is 200"),
        ("text_size = 40", "layout.text_size"),
        ("columns = 3", "layout.columns"),
        ("columns = 2", "need separate rows"),
        ("surface = \"glass\"", "\"plain\" or \"card\""),
        ("animation = \"spin\"", "\"rise\", \"fade\", \"slide\", \"drop\" or \"none\""),
        ("animation_speed = 10.0", "layout.animation_speed"),
        ("panel = \"left\"\nwidth = 500\npanel_size = 400", "less than 300 points"),
        ("panel = \"left\"\npanel_image = \"missing.png\"", "layout.panel_image"),
        ("header_width = 2", "unknown field `header_width`"),
    ];
    for (extra, expected) in cases {
        let message = format!("{:#}", parse(&with(&format!("[layout]\n{extra}")), &dir).unwrap_err());
        assert!(message.contains(expected), "{extra:?} said: {message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
