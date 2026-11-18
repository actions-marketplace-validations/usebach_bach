use super::*;

/// WCAG relative luminance of an opaque colour.
fn luminance(color: Color) -> f64 {
    let channel = |shift: u32| {
        let value = f64::from((color.rgb_value() >> shift) & 0xff) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: Color, b: Color) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `top` painted over the opaque `under`.
fn over(top: Color, under: Color) -> Color {
    let alpha = top.alpha();
    let mix = |shift: u32| {
        let t = ((top.rgb_value() >> shift) & 0xff) as f32;
        let u = ((under.rgb_value() >> shift) & 0xff) as f32;
        ((t * alpha + u * (1.0 - alpha)).round() as u32) << shift
    };
    Color::rgb(mix(16) | mix(8) | mix(0))
}

/// The ink the runtime puts on an accent that names none.
fn ink(accent: Color) -> Color {
    if luminance(accent) < 0.45 {
        Color::rgb(0xffffff)
    } else {
        Color::rgb(0x0f0f0f)
    }
}

#[test]
fn bach_comes_first_and_names_are_unique() {
    assert_eq!(PRESETS[0].name, DEFAULT);
    for (index, preset) in PRESETS.iter().enumerate() {
        assert_eq!(preset.name, preset.name.to_ascii_lowercase());
        assert!(PRESETS[..index].iter().all(|other| other.name != preset.name), "{} twice", preset.name);
        assert!(preset.description.ends_with('.'), "{}: a description is a sentence", preset.name);
        assert!(!preset.fonts.is_empty() && !preset.heading_fonts.is_empty());
        assert!((100..=900).contains(&preset.heading_weight));
    }
    assert!(PRESETS.len() > 10);
}

#[test]
fn every_palette_reads_well() {
    let mut problems = Vec::new();
    for preset in &PRESETS {
        for (mode, palette) in [("light", &preset.light), ("dark", &preset.dark)] {
            let name = format!("{} {mode}", preset.name);
            let background = palette.background.unwrap_or_else(|| panic!("{name}: no background"));
            let text = palette.text.unwrap_or_else(|| panic!("{name}: no text"));
            let muted = palette.muted_text.unwrap_or_else(|| panic!("{name}: no muted text"));
            let accent = palette.accent.unwrap_or_else(|| panic!("{name}: no accent"));
            assert!(palette.border.is_some() && palette.error.is_some(), "{name}: border and error");
            assert_eq!(background.alpha(), 1.0, "{name}: the window is opaque");

            // Text sits on the window (both ends of a gradient) and on cards.
            let mut grounds = vec![background];
            grounds.extend(palette.background_end);
            grounds.extend(palette.card.map(|card| over(card, background)));
            for ground in grounds {
                if contrast(text, ground) < 7.0 {
                    problems.push(format!("{name}: text on {} is {:.2}", ground.hex(), contrast(text, ground)));
                }
                if contrast(muted, ground) < 4.5 {
                    problems.push(format!("{name}: muted text on {} is {:.2}", ground.hex(), contrast(muted, ground)));
                }
            }
            let label = palette.accent_text.unwrap_or_else(|| ink(accent));
            if contrast(label, accent) < 4.5 {
                problems.push(format!("{name}: button label is {:.2}", contrast(label, accent)));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{}",
        problems.join(
            "
"
        )
    );
}

#[test]
fn colours_parse_and_print() {
    assert_eq!(Color::parse("#2563EB"), Some(Color(0x2563ebff)));
    assert_eq!(Color::parse("ffffff14"), Some(Color(0xffffff14)));
    assert_eq!(Color::parse("#fff"), None);
    assert_eq!(Color::parse("#gg0000"), None);
    assert_eq!(Color(0x2563ebff).hex(), "#2563eb");
    assert_eq!(Color(0xffffff14).hex(), "#ffffff14");
}

#[test]
fn names_are_found_in_any_case_and_near_misses_suggested() {
    assert_eq!(preset(" Arcade ").map(|preset| preset.name), Some("arcade"));
    assert!(preset("arcde").is_none());
    assert_eq!(closest("arcde"), Some("arcade"));
    assert_eq!(closest("Terminl"), Some("terminal"));
    assert_eq!(closest("vaporwave"), None);
}

fn look(style: Option<&str>) -> Look {
    Look { style: style.map(String::from), ..Look::default() }
}

#[test]
fn resolving_uses_the_preset_or_the_default() {
    let plain = resolve(&look(None), None);
    assert_eq!(plain.name, "bach");
    assert_eq!(plain.fonts, ["Inter"]);
    assert_eq!(plain.radius, 12.0);
    assert_eq!(resolve(&look(Some("no-such-style")), None).name, "bach");
    let arcade = resolve(&look(Some("arcade")), None);
    assert_eq!(arcade.heading_fonts, ["Bahnschrift"]);
    assert_eq!(arcade.dark.mark, arcade.dark.accent);
    // A preview can try another style over the one the look names.
    assert_eq!(resolve(&look(Some("arcade")), Some("paper")).name, "paper");
}

#[test]
fn the_developers_changes_win() {
    let mut look = look(Some("arcade"));
    look.accent = Some("#2563eb".into());
    let arcade = resolve(&look, None);
    assert_eq!(arcade.dark.accent, Some(Color::rgb(0x2563eb)));
    // The preset's ink was for its own accent; the new one gets its own, and the mark follows it.
    assert_eq!(arcade.dark.accent_text, None);
    assert_eq!(arcade.dark.mark, Some(Color::rgb(0x2563eb)));

    look.custom = Some(Custom {
        font: Some("Nunito".into()),
        radius: Some(4),
        background_image_opacity: Some(3.0),
        dark: Colors { background: Some("#000000".into()), mark: Some("#ffffff".into()), ..Colors::default() },
        ..Custom::default()
    });
    let custom = resolve(&look, None);
    assert_eq!(custom.fonts, ["Nunito", "Inter"]);
    assert_eq!(custom.radius, 4.0);
    assert_eq!(custom.control_radius, 10.0);
    assert_eq!(custom.image_opacity, 1.0);
    assert_eq!(custom.dark.background, Some(Color::rgb(0)));
    // A plain background replaces the preset's gradient.
    assert_eq!(custom.dark.background_end, None);
    assert_eq!(custom.dark.mark, Some(Color::rgb(0xffffff)));
    // The light palette keeps the preset's gradient.
    assert!(custom.light.background_end.is_some());
}

#[test]
fn a_new_background_drops_the_hand_tuned_tokens() {
    let mut look = look(None);
    look.custom = Some(Custom { light: Colors { background: Some("#fdf6e3".into()), ..Colors::default() }, ..Custom::default() });
    let resolved = resolve(&look, None);
    assert_eq!(resolved.light.well, None);
    assert_eq!(resolved.dark.well, PRESETS[0].dark.well);
}
