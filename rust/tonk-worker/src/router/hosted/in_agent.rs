//! Tonk's in-agent look: the one stylesheet every card Tonk draws inside
//! someone's agent app wears (gooey's `in-agent.css` with `tonk-look.css`),
//! and the fonts it names.
//!
//! An MCP App resource is a single HTML document, so each card carries the
//! stylesheet inline. A card marks where it goes with [`MARK`] in its
//! `<head>`; [`splice`] puts the font preloads and the `<style>` there.
//! Its tokens live on `.tonk`, never `:root`, so nothing reaches the agent
//! app or a space's view.
//!
//! The fonts are served from the provider's own origin under
//! [`FONTS_PATH`] with CORS, so a card that names the origin in its
//! resource policy (`resourceDomains`, `openai/widgetCSP`) can load them.

/// Where a card's `<head>` takes the stylesheet.
pub const MARK: &str = "<!-- tonk:in-agent -->";

/// Where the provider serves the fonts (`rust/tonk-ui/assets/in-agent`).
pub const FONTS_PATH: &str = "/in-agent/fonts";

/// The stylesheet, its font addresses still to fill in.
const STYLESHEET: &str = include_str!("in-agent/in-agent.css");

/// What every card opens with: Plex Sans Condensed 400 and 600, and Plex
/// Sans 400. Each tool call is a fresh frame, so without a preload each
/// card lays out on a fallback and then swaps.
const PRELOADS: [&str; 3] = [
    "ibm-plex-sans-condensed-latin-400-normal.woff2",
    "ibm-plex-sans-condensed-latin-600-normal.woff2",
    "ibm-plex-sans-latin-400-normal.woff2",
];

/// The stylesheet with its fonts at `origin`.
pub fn stylesheet(origin: &str) -> String {
    STYLESHEET.replace("__TONK_IN_AGENT_FONTS__", &fonts(origin))
}

/// `html` with [`MARK`] replaced by the font preloads and the stylesheet
/// for `origin`. A document without the mark comes back unchanged.
pub fn splice(html: &str, origin: &str) -> String {
    let fonts = fonts(origin);
    let mut head = String::new();
    for font in PRELOADS {
        head.push_str(&format!(
            "<link rel=\"preload\" href=\"{fonts}/{font}\" as=\"font\" type=\"font/woff2\" \
             crossorigin />\n"
        ));
    }
    head.push_str("<style>\n");
    head.push_str(&stylesheet(origin));
    head.push_str("</style>");
    html.replacen(MARK, &head, 1)
}

fn fonts(origin: &str) -> String {
    format!("{}{FONTS_PATH}", origin.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://tonk.example/";

    #[dialog_common::test]
    fn it_sets_tokens_on_tonk_never_root() {
        let css = stylesheet(ORIGIN);
        let rules: String = css
            .split("/*")
            .map(|part| part.split_once("*/").map_or(part, |(_, rest)| rest))
            .collect();
        assert!(!rules.contains(":root"), "a rule names :root");
        assert!(rules.contains(".tonk, .tonk-hub"));
        assert!(rules.contains("--tonk-text:"));
    }

    #[dialog_common::test]
    fn it_names_every_font_at_the_origin_with_swap() {
        let css = stylesheet(ORIGIN);
        assert!(!css.contains("__TONK_IN_AGENT_FONTS__"));
        assert!(
            !css.contains("@import \""),
            "an import the card cannot load"
        );
        assert!(!css.contains("fonts.googleapis.com"));
        for font in [
            "ibm-plex-sans-condensed-latin-400-normal.woff2",
            "ibm-plex-sans-condensed-latin-500-normal.woff2",
            "ibm-plex-sans-condensed-latin-600-normal.woff2",
            "ibm-plex-sans-latin-400-normal.woff2",
            "ibm-plex-mono-latin-400-normal.woff2",
        ] {
            assert!(
                css.contains(&format!(
                    "url(\"https://tonk.example/in-agent/fonts/{font}\")"
                )),
                "{font} is not at the origin"
            );
        }
        // Each face that loads a file swaps; the local fallbacks load none.
        for face in css.split("@font-face").skip(1) {
            let face = face.split_once('}').map_or(face, |(face, _)| face);
            if face.contains("url(") {
                assert!(face.contains("font-display: swap"), "{face}");
            }
        }
    }

    #[dialog_common::test]
    fn it_preloads_the_three_opening_faces_with_crossorigin() {
        let html = splice("<head>\n<!-- tonk:in-agent -->\n</head>", ORIGIN);
        assert!(!html.contains(MARK));
        assert_eq!(html.matches("rel=\"preload\"").count(), 3);
        assert_eq!(html.matches("crossorigin").count(), 3);
        assert!(html.contains(
            "href=\"https://tonk.example/in-agent/fonts/ibm-plex-sans-condensed-latin-600-normal.woff2\""
        ));
        assert!(html.contains("<style>"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    fn it_ships_every_font_the_stylesheet_names() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tonk-ui/assets/in-agent/fonts");
        let css = stylesheet("");
        for name in css
            .split("url(\"/in-agent/fonts/")
            .skip(1)
            .filter_map(|rest| rest.split_once('"').map(|(name, _)| name))
        {
            assert!(assets.join(name).is_file(), "{name} is not shipped");
        }
        assert!(
            assets.join("OFL.txt").is_file(),
            "the fonts' licence is not shipped"
        );
    }
}
