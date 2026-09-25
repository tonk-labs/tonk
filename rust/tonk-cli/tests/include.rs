//! `!include` in a document evaluated from a file: the reference
//! resolves against the file's location and the included content is
//! written as the field's value. Inline text has no location, so an
//! include in it is refused.

mod common;

use anyhow::Result;
use tonk_cli::eval::{self, Source};

use crate::common::TestSite;

mod when_the_document_is_a_file {
    use super::*;

    #[dialog_common::test]
    async fn it_inlines_a_sibling_file_as_text() -> Result<()> {
        let test = TestSite::new().await?;
        let dir = test.parent.join("notes");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("body.md"), "# Included\n\nFrom a sibling file.\n")?;
        let document = dir.join("note.yaml");
        std::fs::write(
            &document,
            "xyz.example!:\n  this: id:note\n  body: !include ./body.md\n",
        )?;

        eval::run_against_site(&test.site, Source::File(document), eval::Options::default())
            .await?;

        let out = test
            .eval_inline("xyz.example:\n  this: id:note\n  body: ?body\n")
            .await?;
        assert!(
            out.stdout.contains("From a sibling file."),
            "included text should be stored as the value:\n{}",
            out.stdout
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_an_include_that_does_not_exist() -> Result<()> {
        let test = TestSite::new().await?;
        let document = test.parent.join("note.yaml");
        std::fs::write(
            &document,
            "xyz.example!:\n  this: id:note\n  body: !include ./missing.md\n",
        )?;

        let err =
            eval::run_against_site(&test.site, Source::File(document), eval::Options::default())
                .await
                .unwrap_err();
        assert!(matches!(err, eval::EvalError::Parse(_)), "{err}");
        assert!(err.to_string().contains("missing.md"), "{err}");
        Ok(())
    }
}

mod when_the_document_is_inline {
    use super::*;

    #[dialog_common::test]
    async fn it_refuses_to_include() -> Result<()> {
        let test = TestSite::new().await?;
        let err = test
            .eval_inline("xyz.example!:\n  this: id:note\n  body: !include ./body.md\n")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("cannot be resolved"),
            "an inline document has no location to include from: {err}"
        );
        Ok(())
    }
}
