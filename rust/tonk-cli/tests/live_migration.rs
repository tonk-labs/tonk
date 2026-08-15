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

/// Range predicates select real rows on real storage.
///
/// Constraints live in premise position, so the surface that exercises
/// them is a rule body: three counters are written, and a rule bounded
/// by `>` and `<=` derives an alert for exactly the ones inside the
/// interval. The analyzer test proves the predicates lift; this proves
/// they select.
#[dialog_common::test]
async fn it_selects_rows_with_range_predicates() -> Result<()> {
    let test = common::TestSite::new().await?;

    test.eval_inline(
        r#"concept!: &counter
  with:
    count:
      the: live.check/counter-count
      as: unsigned-integer
      cardinality: one
      description: "count"

concept!: &alert
  with:
    count:
      the: live.check/alert-count
      as: unsigned-integer
      cardinality: one
      description: "count"
"#,
    )
    .await?;

    // The half-open interval (1, 100]: excludes the open lower bound,
    // keeps the inclusive upper one. `>` needs quoting — bare `>` opens
    // a YAML folded scalar — while `<=` is a plain scalar.
    test.eval_inline(
        r#"rule!:
  assert!: alert
  when:
    - assert: counter
      where: { this: ?this, count: ?count }
    - assert: ">"
      where: { of: ?count, with: 1 }
    - assert: <=
      where: { of: ?count, with: 100 }
"#,
    )
    .await?;

    for (anchor, count) in [("low", 1u32), ("mid", 10), ("high", 100)] {
        test.eval_inline(&format!("counter!: &{anchor}\n  count: {count}\n"))
            .await?;
    }

    let alerts = test
        .eval_inline("alert:\n  this: ?this\n  count: ?count\n")
        .await?
        .stdout;

    assert!(
        alerts.contains("10") && alerts.contains("100"),
        "the interval (1, 100] must alert on 10 and 100; saw:\n{alerts}"
    );
    assert!(
        !alerts.contains("count: 1\n"),
        "the interval (1, 100] must exclude the open lower bound 1; saw:\n{alerts}"
    );

    Ok(())
}

/// A `tree/*` resolver is usable as a top-level query head, not just
/// inside a rule premise.
///
/// The inspector reads the store's structure through these; going
/// through the evaluate endpoint (rather than the worker's bespoke
/// `tree/*` formula interception) is what makes tree state ordinary
/// queryable data — joinable, subscribable, composable.
#[dialog_common::test]
async fn it_answers_a_top_level_resolver_query() -> Result<()> {
    let test = common::TestSite::new().await?;

    // Something durable, so the tree has a root worth describing.
    test.eval_inline(
        r#"concept!: &marker
  with:
    tag:
      the: live.check/marker-tag
      as: text
      cardinality: one
      description: "tag"
"#,
    )
    .await?;
    test.eval_inline("marker!: &one\n  tag: \"hi\"\n").await?;

    // The resolver selects by content address, so its `of` must be
    // bound. A document reaches the root by joining through the
    // branch's revision; here the reference is read back directly so
    // the test pins the resolver, not the join.
    let root = test.tree_root().await?;
    let rows = test
        .eval_inline(&format!("tree/node:\n  of: \"{root}\"\n  kind: ?kind\n"))
        .await?
        .stdout;

    // The root of a tree holding real facts is a branch node, and the
    // resolver must bind `?kind` to say so. Asserting the resolved
    // value (not merely that some row came back) is what makes this
    // fail if the resolver is reached but answers with nothing.
    assert!(
        rows.contains(r#"kind: "index""#),
        "the resolver must bind ?kind to the root node's kind; saw:\n{rows}"
    );
    assert!(
        rows.contains(&root),
        "the row must describe the node that was asked for; saw:\n{rows}"
    );

    Ok(())
}
