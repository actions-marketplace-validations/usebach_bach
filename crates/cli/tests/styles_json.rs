//! `bach styles --json` and `bach layouts --json` carry what the designer on
//! usebach.app needs to draw the window: every style's colours and corners and
//! every layout's settings.

mod common;
use common::{bach, text};

fn json_of(command: &str) -> serde_json::Value {
    let out = bach().args([command, "--json"]).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    serde_json::from_str(&text(&out.stdout)).unwrap()
}

fn is_hex(colour: &str) -> bool {
    colour.starts_with('#') && matches!(colour.len(), 7 | 9) && colour[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn every_style_comes_with_its_colours_for_light_and_dark() {
    let styles = json_of("styles");
    let styles = styles["styles"].as_array().unwrap();
    assert_eq!(styles.len(), bach_pack::style::PRESETS.len());
    for style in styles {
        let name = style["name"].as_str().unwrap();
        assert!(style["radius"].is_u64() && style["control_radius"].is_u64() && style["heading_weight"].is_u64(), "{name}");
        assert!(style["fonts"].is_array() && style["heading_fonts"].is_array(), "{name}");
        for mode in ["light", "dark"] {
            let palette = style[mode].as_object().unwrap_or_else(|| panic!("{name} has no {mode} palette"));
            for key in ["background", "text", "accent"] {
                assert!(palette.contains_key(key), "{name} {mode} has no {key}");
            }
            for (key, colour) in palette {
                assert!(is_hex(colour.as_str().unwrap()), "{name} {mode} {key} is {colour}");
            }
        }
    }
    // Bach's own look is the one the designer starts from.
    assert_eq!(styles[0]["name"], "bach");
    assert_eq!(styles[0]["light"]["background"], "#ffffff");
}

#[test]
fn every_layout_comes_with_what_it_sets() {
    let layouts = json_of("layouts");
    let layouts = layouts["layouts"].as_array().unwrap();
    assert_eq!(layouts.len(), bach_pack::style::layout::LAYOUT_PRESETS.len());
    for layout in layouts {
        let name = layout["name"].as_str().unwrap();
        let settings = layout["settings"].as_object().unwrap_or_else(|| panic!("{name} has no settings"));
        assert_eq!(settings.get("preset").and_then(|preset| preset.as_str()), Some(name));
    }
    let showcase = layouts.iter().find(|layout| layout["name"] == "showcase").unwrap();
    assert_eq!(showcase["settings"]["panel"], "right", "a panel's side is a setting the designer draws");
}
