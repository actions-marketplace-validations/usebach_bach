use super::*;

#[test]
fn no_layout_is_the_window_bach_always_drew() {
    let plain = Layout::default().resolve();
    assert_eq!((plain.width, plain.height, plain.panel), (540.0, None, Panel::None));
    assert_eq!(plain.padding, [44.0, 32.0, 24.0, 32.0]);
    assert_eq!(plain.align, Align::Center);
    assert_eq!(plain.row_height(), 73.0, "the row height the window always grew by");
    assert_eq!((plain.animation, plain.animation_speed), (Animation::Rise, 1.0));
    assert_eq!(preset("classic").unwrap().1.resolve(), plain);
}

#[test]
fn a_panel_changes_its_defaults() {
    let picture = Layout { panel: Some(Panel::Right), ..Layout::default() }.resolve();
    assert_eq!((picture.width, picture.header, picture.align), (880.0, HeaderPlace::Content, Align::Left));
    assert_eq!(picture.panel_size, 405.0);
    let titled = Layout { panel: Some(Panel::Left), header: Some(HeaderPlace::Panel), ..Layout::default() }.resolve();
    assert_eq!((titled.panel_size, titled.gap), (352.0, 0.0));
    let narrow = Layout { panel: Some(Panel::Left), width: Some(500), panel_size: Some(600), ..Layout::default() }.resolve();
    assert_eq!(narrow.panel_size, 200.0, "the choices keep room");
    let banner = Layout { panel: Some(Panel::Top), ..Layout::default() }.resolve();
    assert_eq!((banner.width, banner.panel_size), (600.0, 190.0));
    // A header on no panel stays with the choices.
    assert_eq!(Layout { header: Some(HeaderPlace::Panel), ..Layout::default() }.resolve().header, HeaderPlace::Content);
}

#[test]
fn every_preset_resolves_and_the_developer_wins() {
    assert!(LAYOUT_PRESETS.len() >= 10);
    for entry in &LAYOUT_PRESETS {
        let (found, layout) = preset(&entry.name.to_uppercase()).unwrap();
        assert_eq!(found.name, entry.name);
        assert_eq!(layout.preset.as_deref(), Some(entry.name));
        assert!(entry.description.ends_with('.'), "{}", entry.name);
        let arranged = layout.resolve();
        assert!(arranged.width >= f32::from(bounds::WIDTH.0) && arranged.width <= f32::from(bounds::WIDTH.1));
    }
    let (_, showcase) = preset("showcase").unwrap();
    let own = Layout { panel: Some(Panel::Left), heading_size: Some(40), ..Layout::default() };
    let mixed = showcase.with(&own).resolve();
    assert_eq!((mixed.panel, mixed.heading_size, mixed.panel_radius), (Panel::Left, 40.0, 26.0));
    assert_eq!(closest("showcas"), Some("showcase"));
    assert_eq!(closest("carousel"), None);
}

#[test]
fn the_manifest_keeps_only_what_was_set() {
    let layout = Layout { buttons: Some(Buttons::Stretch), padding: Some([8, 8, 8, 8]), ..Layout::default() };
    let json = serde_json::to_string(&layout).unwrap();
    assert_eq!(json, r#"{"padding":[8,8,8,8],"buttons":"stretch"}"#);
    assert_eq!(serde_json::from_str::<Layout>(&json).unwrap(), layout);
}
