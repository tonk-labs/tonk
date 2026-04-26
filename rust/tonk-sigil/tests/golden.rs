use tonk_sigil::Sigil;

#[test]
fn zero_renders_all_zeros() {
    let html = Sigil::from(0u32).render();
    assert!(html.contains("sfx-00"));
    assert!(html.contains("pfx-00"));
    // 4 cells: two sfx (positions 0,2) + two pfx (positions 1,3)
    assert_eq!(html.matches("sfx-00").count(), 4); // 2 cells × 2 mask refs (mask-image + -webkit-mask-image)
    assert_eq!(html.matches("pfx-00").count(), 4);
}

#[test]
fn default_big_endian_byte_order() {
    let html = Sigil::from(0xdeadbeefu32).render();
    // Bytes in BE: de, ad, be, ef
    // Positions: 0=sfx, 1=pfx, 2=sfx, 3=pfx
    assert!(html.contains("sfx-de"));
    assert!(html.contains("pfx-ad"));
    assert!(html.contains("sfx-be"));
    assert!(html.contains("pfx-ef"));
}

#[test]
fn renders_a_2x2_grid() {
    let html = Sigil::from(0u32).render();
    // Wrapper div + 4 cell divs = 5 `<div`
    assert_eq!(html.matches("<div").count(), 5);
    assert!(html.contains("grid-template-columns:1fr 1fr"));
    assert!(html.contains("grid-template-rows:1fr 1fr"));
}

#[test]
fn cells_use_mask_image() {
    let html = Sigil::from(0u32).render();
    assert!(html.contains("mask-image:url("));
    assert!(html.contains("-webkit-mask-image:url("));
    assert!(html.contains("mask-mode:luminance"));
}

#[test]
fn default_omits_inline_color_vars() {
    // Leaves `--sigil-fg` unset on the wrapper so ancestor CSS
    // cascades through to each cell's `currentColor`.
    let html = Sigil::from(0u32).render();
    assert!(!html.contains("--sigil-fg"));
}

#[test]
fn fill_override_sets_inline_var() {
    let html = Sigil::from(0u32).fill("purple").render();
    assert!(html.contains("--sigil-fg:purple"));
    assert!(html.contains("color:var(--sigil-fg)"));
}

#[test]
fn default_sprite_href_is_assets_relative() {
    let html = Sigil::from(0u32).render();
    assert!(html.contains("/sigils.svg#"));
}

#[test]
fn sprite_href_builder_overrides() {
    let html = Sigil::from(0u32)
        .sprite_href("/assets/tonk-sigils.svg")
        .render();
    assert!(html.contains("/assets/tonk-sigils.svg#"));
    assert!(!html.contains("/sigils.svg#"));
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
fn rendering_is_responsive() {
    let html = Sigil::from(0u32).render();
    // Wrapper fills its CSS box.
    assert!(html.contains("width:100%"));
    assert!(html.contains("height:100%"));
}

#[test]
fn display_matches_render() {
    let s = Sigil::from(0xdeadbeefu32);
    assert_eq!(s.to_string(), s.render());
}

#[test]
fn output_is_well_formed_root() {
    let html = Sigil::from(0u32).render();
    assert!(html.starts_with("<div"));
    assert!(html.ends_with("</div>"));
}

#[test]
fn max_u32_renders_ff_symbols() {
    let html = Sigil::from(u32::MAX).render();
    assert!(html.contains("sfx-ff"));
    assert!(html.contains("pfx-ff"));
}
