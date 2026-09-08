//! Real-browser test for the on-demand `/console` library.
//!
//! `console.yaml` is deliberately NOT part of any branch's seed. The route
//! exists only after the service worker, having failed to match `/console`
//! against the profile branch's route table, fetches the served library and
//! evaluates it onto the branch — then matches again.
//!
//! That whole path is invisible to a unit test: the fetch needs a
//! service-worker scope and a server actually serving `/library/console.yaml`.
//! So it is asserted here, against the real built bundle, by the only
//! evidence that covers every step at once — the page renders.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use thirtyfour::prelude::*;

    use crate::helpers::{TestEnvironment, goto};

    /// Find `selector`, retrying until it appears or the deadline passes.
    /// The console's first paint waits on a fetch, an evaluate, a commit and
    /// a subscription frame, so a bare `find` races the install.
    async fn element(driver: &WebDriver, selector: &str) -> Result<WebElement> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(element) => return Ok(element),
                Err(error) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(anyhow!("{selector} never appeared: {error}"));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Enter the sealed guest frame the top-level `<tonk-site>` renders into.
    /// The guest is at an opaque origin, so reaching its DOM means switching
    /// the driver's browsing context to it.
    async fn enter_guest(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        let frame = element(driver, "tonk-site > iframe").await?;
        frame.enter_frame().await?;
        Ok(())
    }

    /// Visiting `/console` on a profile that has never seen the console
    /// library renders the console page.
    ///
    /// Every link in the chain has to hold for this to pass: the route misses,
    /// `install_library_for` claims the path, the fetch of
    /// `/library/console.yaml` succeeds inside the SW scope, the document
    /// analyzes and commits onto the profile branch, the re-match finds the
    /// freshly-committed route, the site stamps, and the guest's display
    /// resolves `console/route` to its view. Asserting the rendered heading
    /// rather than the underlying fact is deliberate — the fact being present
    /// while the page stays blank is exactly the failure worth catching.
    #[dialog_common::test]
    async fn it_installs_the_console_library_on_first_visit(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        goto(&driver, env.tonk_web.join("console")?.as_str()).await?;
        enter_guest(&driver).await?;

        let heading = element(&driver, ".console__head").await?;
        assert_eq!(
            heading.text().await?.trim(),
            "Console",
            "the console page must render its heading — a blank frame means \
             the library never installed or its view never resolved"
        );

        // The subscriptions tree, and at least one group in it. The page
        // always has something to show: rendering it opens a subscription of
        // its own, so an empty tree here means the publisher never ran.
        element(&driver, "wa-tree").await?;
        let group = element(&driver, ".console-group__space").await?;
        assert!(
            !group.text().await?.trim().is_empty(),
            "a group row must name the repository its subscriptions belong to"
        );

        // The query, rendered as highlighted notation rather than raw JSON.
        // The decoration spans are the evidence `<tonk-notation>` actually
        // tokenized it — text alone would also appear if it had rendered the
        // source verbatim.
        let notation = element(&driver, "tonk-notation .tonk-notation-pre").await?;
        let rendered = notation.text().await?;
        assert!(
            !rendered.contains('{'),
            "the query must be notation, not an uninterpolated template or JSON: {rendered}"
        );
        assert!(
            !rendered.contains("!:"),
            "a subscription READS, so its notation must not carry the `!` \
             mutation marker: {rendered}"
        );
        element(&driver, "tonk-notation .tonk-cm-key").await?;

        // The opened-at stamp. `<wa-relative-time>` renders nothing at all
        // unless it is registered in the guest bundle AND got a parseable
        // date, so non-empty text covers both.
        let opened = driver
            .execute(
                r#"const el = document.querySelector("wa-relative-time");
                   if (!el) return "";
                   return (el.shadowRoot ? el.shadowRoot.textContent : el.textContent).trim();"#,
                vec![],
            )
            .await?;
        assert!(
            !opened.json().as_str().unwrap_or("").is_empty(),
            "each subscription must show when it was opened"
        );

        driver.quit().await?;
        Ok(())
    }

    /// A second visit still renders — the install is idempotent.
    ///
    /// The first visit committed the library's concepts, views and routes onto
    /// the branch. The second must match on the first try and render exactly
    /// the same page; a re-evaluation that minted duplicate rows, or a
    /// re-declared `tonk:route` that landed as a different concept, would show
    /// up here as a broken or doubled page rather than a clean one.
    #[dialog_common::test]
    async fn it_renders_the_console_again_on_a_later_visit(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;

        goto(&driver, env.tonk_web.join("console")?.as_str()).await?;
        enter_guest(&driver).await?;
        element(&driver, ".console__head").await?;

        // Leave and come back, so the second visit routes against a branch
        // that already carries the library.
        driver.enter_default_frame().await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        goto(&driver, env.tonk_web.join("console")?.as_str()).await?;
        enter_guest(&driver).await?;

        let headings = driver.find_all(By::Css(".console__head")).await?;
        assert_eq!(
            headings.len(),
            1,
            "the console must render exactly one heading on a repeat visit; \
             more than one means the re-install duplicated the view"
        );

        driver.quit().await?;
        Ok(())
    }
}
