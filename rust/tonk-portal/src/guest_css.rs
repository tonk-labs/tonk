//! The CSS a sealed guest is handed: one inline stylesheet, so every address
//! in it must stand on its own, and every face the host can reach is inlined
//! (an opaque-origin guest can't fetch a font).

/// The directories whose files the host inlines into a guest's CSS: the
/// app's own `/fonts/`, and `/in-agent/fonts/`, where Tonk's own pages in a
/// browser (`/in-browser/fonts.css`) find their faces.
pub(crate) const FONT_DIRS: [&str; 2] = ["/fonts/", "/in-agent/fonts/"];

/// Make every relative `url(...)` argument in `css` absolute, resolved
/// against `sheet`, the path the stylesheet was served at. A guest's CSS is
/// inline, so an address relative to the file it came from would resolve
/// against nothing; absolute, it is one the font inlining recognises.
/// Absolute paths, schemes (`data:`, `https:`) and fragments are kept.
pub(crate) fn absolute_urls(css: &str, sheet: &str) -> String {
    let base: Vec<&str> = {
        let path = sheet.split(['?', '#']).next().unwrap_or_default();
        let mut parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        parts.pop();
        parts
    };
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("url(") {
        let (head, tail) = rest.split_at(i + "url(".len());
        out.push_str(head);
        let Some(close) = tail.find(')') else {
            rest = tail;
            break;
        };
        let raw = &tail[..close];
        let arg = raw.trim().trim_matches(['"', '\'']);
        let relative =
            !arg.is_empty() && !arg.starts_with('/') && !arg.starts_with('#') && !arg.contains(':');
        if relative {
            let mut parts = base.clone();
            for part in arg.split('/') {
                match part {
                    "" | "." => {}
                    ".." => {
                        parts.pop();
                    }
                    part => parts.push(part),
                }
            }
            out.push_str(&raw.replacen(arg, &format!("/{}", parts.join("/")), 1));
        } else {
            out.push_str(raw);
        }
        rest = &tail[close..];
    }
    out.push_str(rest);
    out
}

/// Collect the distinct font paths (under [`FONT_DIRS`]) referenced as `url(...)` arguments
/// in `css`, in first-appearance order. Only genuine `url()` arguments
/// qualify: scanning for the raw `/fonts/` substring also matches prose in
/// comments — a comment in styles.css mentioning `` `/fonts/` `` used to
/// produce a junk `GET /fonts/%60%20(copied…` 404 on every guest boot.
pub(crate) fn find_font_paths(css: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut rest = css;
    while let Some(i) = rest.find("url(") {
        rest = &rest[i + "url(".len()..];
        let Some(close) = rest.find(')') else { break };
        let arg = rest[..close].trim().trim_matches(['"', '\'']);
        // Skip a bare font directory with no filename.
        if FONT_DIRS
            .iter()
            .any(|dir| arg.strip_prefix(dir).is_some_and(|name| !name.is_empty()))
            && !paths.iter().any(|p| p == arg)
        {
            paths.push(arg.to_owned());
        }
        rest = &rest[close + 1..];
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    // --- find_font_paths: url() argument extraction --------------------

    #[dialog_common::test]
    fn it_finds_font_paths_in_url_args_only() {
        // The comment reproduces styles.css prose that the old raw-substring
        // scan turned into `GET /fonts/%60%20(copied…` — a 404 on every
        // guest boot. Prose mentions of `/fonts/` must not count; quoted,
        // single-quoted, and bare url() arguments must; duplicates collapse.
        let css = r#"
            /* Files live under `/fonts/` (copied into the dist by Trunk's
               `copy-dir` on `./assets/fonts`). */
            @font-face { src: url("/fonts/space-grotesk-400.woff2") format("woff2"); }
            @font-face { src: url('/fonts/gestalte-400.otf'); }
            .bare { mask: url(/fonts/unquoted.ttf); }
            .other { background: url("/images/mark-white.svg"); }
            .dup { src: url("/fonts/space-grotesk-400.woff2"); }
            .empty { background: url("/fonts/"); }
        "#;
        assert_eq!(
            find_font_paths(css),
            vec![
                "/fonts/space-grotesk-400.woff2",
                "/fonts/gestalte-400.otf",
                "/fonts/unquoted.ttf",
            ]
        );
    }

    #[dialog_common::test]
    fn it_finds_the_in_browser_pages_faces() {
        let css = r#"@font-face { src: url("/in-agent/fonts/ibm-plex-sans-latin-400-normal.woff2"); }
            .not { background: url("/in-agent/other.svg"); }"#;
        assert_eq!(
            find_font_paths(css),
            vec!["/in-agent/fonts/ibm-plex-sans-latin-400-normal.woff2"]
        );
    }

    #[dialog_common::test]
    fn it_resolves_a_sheets_relative_urls_against_where_it_was_served() {
        let css = r#"a { src: url("../in-agent/fonts/a.woff2") format("woff2"); }
            b { mask: url('./b.svg'); }
            c { mask: url(c.svg); }
            d { mask: url("/fonts/d.otf"); }
            e { mask: url("data:image/svg+xml,%3Csvg%3E"); }"#;
        assert_eq!(
            absolute_urls(css, "/in-browser/fonts.css"),
            r#"a { src: url("/in-agent/fonts/a.woff2") format("woff2"); }
            b { mask: url('/in-browser/b.svg'); }
            c { mask: url(/in-browser/c.svg); }
            d { mask: url("/fonts/d.otf"); }
            e { mask: url("data:image/svg+xml,%3Csvg%3E"); }"#
        );
        // A face as gooey's in-browser/fonts.css declares it.
        let face = r#"@font-face { font-family: "IBM Plex Sans"; src: url("../in-agent/fonts/ibm-plex-sans-latin-400-normal.woff2") format("woff2"); }"#;
        assert_eq!(
            find_font_paths(&absolute_urls(face, "/in-browser/fonts.css")),
            vec!["/in-agent/fonts/ibm-plex-sans-latin-400-normal.woff2"]
        );
    }
}
