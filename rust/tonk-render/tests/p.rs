#[test]
fn probe_attr_brace() {
    let r = tonk_render::parse_fragment(
        r#"<article data-model={dom.host/model}><h2>{name}</h2></article>"#,
    );
    eprintln!("UNQUOTED: {:#?}", r);
    let r2 = tonk_render::parse_fragment(
        r#"<article data-model="{dom.host/model}"><h2>{name}</h2></article>"#,
    );
    eprintln!("QUOTED: {:#?}", r2);
}
