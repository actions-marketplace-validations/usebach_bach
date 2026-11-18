//! `[layout]`.

use super::{project_dir, MINIMAL};
use crate::config::parse;
use crate::manifest::LookFileKind;
use crate::style::layout::{Align, Animation, Buttons, Footer, Header, Rows};

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
        header = "Left"
        header_width = 300
        panel_image = "side.jpg"
        align = "left"
        padding = [40, 28]
        heading_size = 28
        text_size = 15
        rows = "separate"
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
    assert_eq!((layout.width, layout.height, layout.header), (Some(820), Some(480), Some(Header::Left)));
    assert_eq!(layout.padding, Some([40, 28, 40, 28]));
    assert_eq!((layout.align, layout.rows, layout.icons), (Some(Align::Left), Some(Rows::Separate), Some(false)));
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
fn layout_mistakes_name_their_setting() {
    let dir = project_dir("layout-mistakes");
    let cases = [
        ("width = 100", "layout.width is 100; use 380 to 1400"),
        ("height = 5000", "layout.height"),
        ("header = \"bottom\"", "use \"top\", \"left\" or \"right\""),
        ("align = \"middle\"", "layout.align"),
        ("padding = [1, 2, 3]", "one number (every side), two"),
        ("padding = 200", "layout.padding is 200"),
        ("text_size = 40", "layout.text_size"),
        ("animation = \"spin\"", "\"rise\", \"fade\", \"slide\", \"drop\" or \"none\""),
        ("animation_speed = 10.0", "layout.animation_speed"),
        ("header_width = 300", "isn't at the side"),
        ("header = \"left\"\nwidth = 500\nheader_width = 400", "less than 220 points"),
        ("header = \"left\"\npanel = false\npanel_image = \"x.png\"", "layout.panel is false"),
        ("header = \"left\"\npanel_image = \"missing.png\"", "layout.panel_image"),
        ("columns = 2", "unknown field `columns`"),
    ];
    for (extra, expected) in cases {
        let message = format!("{:#}", parse(&with(&format!("[layout]\n{extra}")), &dir).unwrap_err());
        assert!(message.contains(expected), "{extra:?} said: {message}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
