//! End-to-end check of the paths the dialog migration rewrote.
//!
//! A real spot on real storage: the standard library seeds at repo
//! creation, a rule installs and fires at commit time through dialog's
//! native induction, and the command that triggered it leaves no trace.
//! This is the headless equivalent of clicking through a running
//! instance — the whole chain, not a unit of it.

mod common;

use anyhow::Result;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_browser);

/// The standard library seeds, a native rule fires on a transient
/// command, and the derived fact is durable while the command itself is
/// swept.
#[dialog_common::test]
async fn it_seeds_installs_a_rule_and_fires_it_on_a_command() -> Result<()> {
    let test = common::TestSite::new().await?;

    // A fresh repo answers the concept-of-concept query, which is the
    // read path every schema view is built on.
    let seeded = test.eval_inline("concept:\n  name: ?name\n").await?.stdout;
    assert!(
        seeded.contains("db:attribute"),
        "a fresh repo must resolve the built-in schema concepts; saw:\n{seeded}"
    );

    // A command (transient) concept and the durable concept a rule
    // derives from it.
    test.eval_inline(
        r#"concept!: &ping
  transient:
  with:
    tag:
      the: live.check/ping-tag
      as: text
      cardinality: one
      description: "tag"

concept!: &pong
  with:
    tag:
      the: live.check/pong-tag
      as: text
      cardinality: one
      description: "tag"
"#,
    )
    .await?;

    test.eval_inline(
        r#"rule!:
  assert!: pong
  when:
    - assert: ping
      where: { this: ?this, tag: ?tag }
"#,
    )
    .await?;

    // Assert the command; commit-time induction fires the rule.
    test.eval_inline(
        r#"ping!: &hello
  tag: "hi"
"#,
    )
    .await?;

    // The derived fact is durable...
    let pong = test
        .eval_inline("pong:\n  this: ?this\n  tag: ?tag\n")
        .await?
        .stdout;
    assert!(
        pong.contains("hi"),
        "the rule must derive a durable pong from the ping command; saw:\n{pong}"
    );

    // ...and the command itself was swept, never committed.
    let ping = test
        .eval_inline("ping:\n  this: ?this\n  tag: ?tag\n")
        .await?
        .stdout;
    assert!(
        !ping.contains("hi"),
        "a command must not persist past the commit that dispatched it; saw:\n{ping}"
    );

    Ok(())
}
