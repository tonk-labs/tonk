//! The head markup an embedder gives a sealed portal.
//!
//! Split out as a pure string builder so it is covered by a native unit
//! test: the elements themselves are `wasm32`-only.

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn head_markup(outer_html: &[String]) -> String {
    outer_html.concat()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_hoists_nothing_when_the_embedder_supplied_nothing() {
        assert_eq!(head_markup(&[]), "");
    }

    #[dialog_common::test]
    fn it_hoists_the_embedders_styles_in_source_order() {
        let markup = head_markup(&[
            "<style>a{color:red}</style>".to_owned(),
            "<style>b{color:blue}</style>".to_owned(),
        ]);
        assert_eq!(
            markup,
            "<style>a{color:red}</style><style>b{color:blue}</style>"
        );
    }
}
