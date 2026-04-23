use tonk_sigil::Sigil;

#[test]
fn zero_renders_all_zeros() {
    let svg = Sigil::from(0u32).render();
    assert!(svg.contains("sfx-00"));
    assert!(svg.contains("pfx-00"));
    // 4 uses total, two of each prefix class
    assert_eq!(svg.matches("<use ").count(), 4);
    assert_eq!(svg.matches("#sfx-00").count(), 2);
    assert_eq!(svg.matches("#pfx-00").count(), 2);
}

#[test]
fn default_big_endian_byte_order() {
    let svg = Sigil::from(0xdeadbeefu32).render();
    // Bytes in BE: de, ad, be, ef
    // Positions: 0=sfx, 1=pfx, 2=sfx, 3=pfx
    assert!(svg.contains("#sfx-de"));
    assert!(svg.contains("#pfx-ad"));
    assert!(svg.contains("#sfx-be"));
    assert!(svg.contains("#pfx-ef"));
}

#[test]
fn grid_positions_translate_correctly() {
    let svg = Sigil::from(0u32).render();
    // 2x2 grid in 128-unit viewBox: cells at (0,0), (64,0), (0,64), (64,64)
    assert!(svg.contains("translate(0 0)"));
    assert!(svg.contains("translate(64 0)"));
    assert!(svg.contains("translate(0 64)"));
    assert!(svg.contains("translate(64 64)"));
}

#[test]
fn viewbox_is_fixed_at_128() {
    let svg = Sigil::from(0u32).render();
    assert!(svg.contains("viewBox=\"0 0 128 128\""));
}

#[test]
fn default_colors_emit_css_vars() {
    let svg = Sigil::from(0u32).render();
    assert!(svg.contains("--sigil-fg:currentColor"));
    assert!(svg.contains("--sigil-bg:transparent"));
}

#[test]
fn custom_colors_override_defaults() {
    let svg = Sigil::from(0u32).fill("black").stroke("white").render();
    assert!(svg.contains("--sigil-fg:black"));
    assert!(svg.contains("--sigil-bg:white"));
}

#[test]
fn default_sprite_href_is_assets_relative() {
    let svg = Sigil::from(0u32).render();
    assert!(svg.contains("href=\"/sigils.svg#"));
}

#[test]
fn sprite_href_builder_overrides() {
    let svg = Sigil::from(0u32)
        .sprite_href("/assets/tonk-sigils.svg")
        .render();
    assert!(svg.contains("href=\"/assets/tonk-sigils.svg#"));
    assert!(!svg.contains("href=\"/sigils.svg#"));
}

#[test]
fn from_bytes_array_equivalent_to_u32() {
    let a = Sigil::from(0x12345678u32).render();
    let b = Sigil::from([0x12, 0x34, 0x56, 0x78]).render();
    assert_eq!(a, b);
}

#[test]
fn distinct_inputs_produce_distinct_output() {
    let a = Sigil::from(0x00000001u32).render();
    let b = Sigil::from(0x00000100u32).render();
    assert_ne!(a, b);
}

#[test]
fn svg_is_responsive() {
    let svg = Sigil::from(0u32).render();
    // No pixel-valued width/height — only 100% for CSS-driven sizing
    assert!(svg.contains("width:100%"));
    assert!(svg.contains("height:100%"));
    assert!(!svg.contains("width=\"128\""));
    assert!(!svg.contains("height=\"128\""));
}

#[test]
fn display_matches_render() {
    let s = Sigil::from(0xdeadbeefu32);
    assert_eq!(s.to_string(), s.render());
}

#[test]
fn svg_is_well_formed_root() {
    let svg = Sigil::from(0u32).render();
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
}

#[test]
fn max_u32_renders_ff_symbols() {
    let svg = Sigil::from(u32::MAX).render();
    assert!(svg.contains("#sfx-ff"));
    assert!(svg.contains("#pfx-ff"));
    assert_eq!(svg.matches("#sfx-ff").count(), 2);
    assert_eq!(svg.matches("#pfx-ff").count(), 2);
}
