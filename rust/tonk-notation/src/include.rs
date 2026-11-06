//! `!include` expansion.
//!
//! The parser records an `!include <reference>` field value as a
//! [`FieldValue::Include`] and does nothing else: it is pure, and
//! loading a resource is IO whose shape depends on the host (a file
//! read in the CLI, a fetch in a browser). [`expand`] is the step
//! that runs between parsing and analysis. It resolves each
//! reference against the document's [`Syntax::base`], asks a
//! [`Load`] implementation for the bytes, and replaces the node with
//! the literal they spell, or, for `!include/asset`, with the
//! `asset:` reference [`Load::store`] filed them under.
//!
//! Included content is inlined as a value, never parsed as notation,
//! so an included file cannot include anything in turn.
//!
//! A pipeline that never expands leaves the nodes in place, and the
//! analyzer refuses them. That is the intended outcome for a document
//! with no location (see [`INLINE_LOCATION`][crate::parse::INLINE_LOCATION]):
//! there is nothing to resolve a relative reference against.

use std::future::Future;

use dialog_common::ConditionalSend;
use lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use url::Url;

use crate::syntax::{Expression, Field, FieldValue, Include, IncludeForm, Scalar, Syntax};

/// Fetches the content an `!include` names.
///
/// A host implements this for the URI schemes it can reach and
/// refuses the rest; the CLI, for example, reads `file:` URIs from
/// disk. The error is a human-readable reason, reported against the
/// `!include` that asked for the resource.
pub trait Load {
    /// Load the resource at `uri`.
    fn load(&self, uri: &Url) -> impl Future<Output = Result<Vec<u8>, String>> + ConditionalSend;

    /// File the `bytes` loaded from `uri` as a content-addressed asset
    /// and return the reference a field holds for it, for
    /// `!include/asset`. The host may store them right away or with the
    /// commit that refers to them.
    ///
    /// A host that cannot store assets keeps the default, which refuses, so
    /// the include is reported rather than silently inlined.
    fn store(
        &self,
        uri: &Url,
        bytes: Vec<u8>,
    ) -> impl Future<Output = Result<String, String>> + ConditionalSend {
        let _ = (uri, bytes);
        std::future::ready(Err("assets cannot be stored here".to_owned()))
    }
}

/// Replace every `!include` in `syntax` with the content it names.
///
/// Returns one diagnostic per include that could not be expanded —
/// a reference with no base to resolve against, a load failure, or
/// `!include` content that is not UTF-8. Those nodes are left as
/// they were, so analysis rejects the document if the caller goes on
/// regardless.
pub async fn expand<L: Load>(syntax: &mut Syntax, loader: &L) -> Vec<Diagnostic> {
    let mut pending = Vec::new();
    for expression in &mut syntax.expressions {
        let application = match expression {
            Expression::Query(application) => application,
            Expression::Claim(claim) => &mut claim.inner,
        };
        collect(&mut application.fields, &mut pending);
    }

    let mut diagnostics = Vec::new();
    for (value, range) in pending {
        let FieldValue::Include(include) = &*value else {
            unreachable!("collect only gathers includes");
        };
        match inline(include, &syntax.base, loader).await {
            Ok(expanded) => *value = expanded,
            Err(message) => diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("tonk-notation".to_owned()),
                message,
                ..Default::default()
            }),
        }
    }
    diagnostics
}

/// Gather every include under `fields`, with the range of the value
/// it sits in, walking nested mappings and rule premises.
fn collect<'a>(fields: &'a mut [Field], out: &mut Vec<(&'a mut FieldValue, Range)>) {
    for field in fields {
        let range = field.value_range;
        if matches!(field.value, FieldValue::Include(_)) {
            out.push((&mut field.value, range));
            continue;
        }
        match &mut field.value {
            FieldValue::Nested(nested) => collect(nested, out),
            FieldValue::Premises(premises) => {
                for premise in premises {
                    collect(&mut premise.bindings, out);
                }
            }
            FieldValue::Include(_)
            | FieldValue::Literal(_)
            | FieldValue::Variable(_)
            | FieldValue::Blank
            | FieldValue::Symbol(_)
            | FieldValue::Uri(_)
            | FieldValue::List(_) => {}
        }
    }
}

async fn inline<L: Load>(include: &Include, base: &Url, loader: &L) -> Result<FieldValue, String> {
    let tag = include.form.tag();
    let uri = include.resolve(base).map_err(|failure| {
        format!(
            "`!{tag} {}` cannot be resolved against this document's location `{base}`: {failure}",
            include.reference
        )
    })?;
    let bytes = loader
        .load(&uri)
        .await
        .map_err(|failure| format!("`!{tag}` could not load `{uri}`: {failure}"))?;
    match include.form {
        IncludeForm::Bytes => Ok(FieldValue::Literal(Scalar::Included(bytes))),
        IncludeForm::Text => String::from_utf8(bytes)
            .map(|text| FieldValue::Literal(Scalar::String(text)))
            .map_err(|_| {
                format!(
                    "`!{tag}` content of `{uri}` is not UTF-8 text; use `!{}` to keep its bytes",
                    IncludeForm::Bytes.tag()
                )
            }),
        IncludeForm::Asset => loader
            .store(&uri, bytes)
            .await
            .map(FieldValue::Uri)
            .map_err(|failure| format!("`!{tag}` could not store `{uri}`: {failure}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::parse::{parse, parse_at};

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// A loader over a fixed set of resources.
    struct Fixtures(HashMap<String, Vec<u8>>);

    impl Fixtures {
        fn new(entries: &[(&str, &[u8])]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(uri, bytes)| (uri.to_string(), bytes.to_vec()))
                    .collect(),
            )
        }
    }

    impl Load for Fixtures {
        async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
            self.0
                .get(uri.as_str())
                .cloned()
                .ok_or_else(|| "not found".to_owned())
        }
    }

    /// A loader that also stores assets, naming each by its length so a
    /// test can see which bytes went in.
    struct Stored(Fixtures);

    impl Load for Stored {
        async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
            self.0.load(uri).await
        }

        async fn store(&self, _uri: &Url, bytes: Vec<u8>) -> Result<String, String> {
            Ok(format!("asset:len{}", bytes.len()))
        }
    }

    fn at(base: &str, text: &str) -> Syntax {
        let parsed = parse_at(Url::parse(base).unwrap(), text);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        parsed.syntax.unwrap()
    }

    fn field<'a>(syntax: &'a Syntax, name: &str) -> &'a FieldValue {
        &syntax.expressions[0]
            .application()
            .fields
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .value
    }

    #[dialog_common::test]
    async fn it_inlines_text_relative_to_the_document() {
        let mut syntax = at(
            "file:///notes/today.yaml",
            "note!:\n  this: ?n\n  body: !include/text ./body.md\n",
        );
        let loader = Fixtures::new(&[("file:///notes/body.md", b"# Hello\n")]);
        let diagnostics = expand(&mut syntax, &loader).await;
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert_eq!(
            field(&syntax, "body"),
            &FieldValue::Literal(Scalar::String("# Hello\n".into()))
        );
    }

    /// Plain `!include` keeps the content as loaded; whether it is text
    /// is the analyzer's call, made from the field's declared type.
    #[dialog_common::test]
    async fn it_keeps_plain_include_content_as_loaded() {
        let mut syntax = at(
            "file:///notes/today.yaml",
            "note!:\n  this: ?n\n  body: !include ./body.md\n",
        );
        let loader = Fixtures::new(&[("file:///notes/body.md", b"# Hello\n")]);
        let diagnostics = expand(&mut syntax, &loader).await;
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert_eq!(
            field(&syntax, "body"),
            &FieldValue::Literal(Scalar::Included(b"# Hello\n".to_vec()))
        );
    }

    #[dialog_common::test]
    async fn it_inlines_bytes_and_nested_values() {
        let mut syntax = at(
            "file:///notes/today.yaml",
            "note!:\n  this: ?n\n  meta:\n    image: !include ../media/a.webp\n",
        );
        let loader = Fixtures::new(&[("file:///media/a.webp", &[0xff, 0x00, 0x7f])]);
        let diagnostics = expand(&mut syntax, &loader).await;
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        let FieldValue::Nested(meta) = field(&syntax, "meta") else {
            panic!("expected nested");
        };
        assert_eq!(
            meta[0].value,
            FieldValue::Literal(Scalar::Included(vec![0xff, 0x00, 0x7f]))
        );
    }

    /// A document with no location has nothing to be relative to, so
    /// the include is refused and left in place for analysis to reject.
    #[dialog_common::test]
    async fn it_refuses_a_relative_include_in_an_inline_document() {
        let mut syntax = parse("note!:\n  this: ?n\n  body: !include ./body.md\n")
            .syntax
            .unwrap();
        let loader = Fixtures::new(&[]);
        let diagnostics = expand(&mut syntax, &loader).await;
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message.contains("cannot be resolved"),
            "{}",
            diagnostics[0].message
        );
        assert!(matches!(field(&syntax, "body"), FieldValue::Include(_)));
    }

    #[dialog_common::test]
    async fn it_reports_load_failures_and_non_text_content() {
        let mut syntax = at(
            "file:///d/doc.yaml",
            "note!:\n  this: ?n\n  missing: !include gone.md\n  binary: !include/text blob.bin\n",
        );
        let loader = Fixtures::new(&[("file:///d/blob.bin", &[0xff, 0xfe])]);
        let diagnostics = expand(&mut syntax, &loader).await;
        let messages: Vec<_> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:#?}");
        assert!(messages[0].contains("could not load `file:///d/gone.md`"));
        assert!(messages[1].contains("not UTF-8"));
    }

    #[dialog_common::test]
    async fn it_replaces_an_asset_include_with_the_stored_reference() {
        let mut syntax = at(
            "file:///site/page.yaml",
            "note!:\n  this: id:page\n  image: !include/asset ./assets/hero.webp\n",
        );
        let loader = Stored(Fixtures::new(&[(
            "file:///site/assets/hero.webp",
            &[0xff, 0x00, 0x7f],
        )]));
        let diagnostics = expand(&mut syntax, &loader).await;
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert_eq!(
            field(&syntax, "image"),
            &FieldValue::Uri("asset:len3".into())
        );
    }

    /// A host that cannot store assets reports the include instead of
    /// quietly inlining the bytes, and leaves it for analysis to reject.
    #[dialog_common::test]
    async fn it_refuses_an_asset_include_where_assets_cannot_be_stored() {
        let mut syntax = at(
            "file:///site/page.yaml",
            "note!:\n  this: id:page\n  image: !include/asset ./hero.webp\n",
        );
        let loader = Fixtures::new(&[("file:///site/hero.webp", &[0x01])]);
        let diagnostics = expand(&mut syntax, &loader).await;
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(
            diagnostics[0].message.contains("cannot be stored"),
            "{}",
            diagnostics[0].message
        );
        assert!(matches!(field(&syntax, "image"), FieldValue::Include(_)));
    }
}
