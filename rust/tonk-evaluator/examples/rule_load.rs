//! A load harness over the standard library's deductive rules.
//!
//! Seeds the profile and notebook libraries into a fresh branch the way
//! the worker does, asserts a population of accounts, spaces and
//! notebook blocks in the shapes the rules read, and times the queries
//! the UI subscribes to: the account's status (four rules electing one
//! case, three of them negated), a space's presence on this device
//! (four rules over the replica facts), and a notebook's block
//! positions (a keyed collection read through the rule's base case).
//! Then it subscribes to each and times the incremental re-poll after
//! a commit that moves a few rows between cases.
//!
//! Sizes and repetitions come from the environment: `ACCOUNTS`,
//! `SPACES`, `BLOCKS`, `RUNS`. Run it against two builds of dialog to
//! compare them:
//!
//! ```sh
//! ACCOUNTS=2000 SPACES=500 BLOCKS=500 cargo run -p tonk-evaluator --example rule_load
//! ```

/// The harness needs tokio and the native test peer, so it is native
/// only; the wasm build of the crate's examples sees an empty program.
#[cfg(not(target_arch = "wasm32"))]
mod load {
    use std::time::{Duration, Instant};

    use dialog_peer::helpers::{test_repo, test_session_with_peer};
    use dialog_query::query::Output as _;
    use dialog_query::{ConceptQuery, Parameters, Term};
    use dialog_repository::Branch;
    use serde_json::json;
    use tonk_evaluator::evaluate::SyntaxEvaluateExt as _;
    use tonk_notation::{expand, parse, parse_at};
    use tonk_schema::concept::QueryPlan;

    const PROFILE_LIBRARY: &str = include_str!("../../tonk-core/assets/library/profile.yaml");
    const NOTEBOOK_LIBRARY: &str = include_str!("../../tonk-core/assets/library/notebook.yaml");

    fn size(name: &str, default: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    /// A one-field concept query, `this` and the field free, as the UI's
    /// subscriptions spell them.
    fn query(field: &str, the: &str, kind: &str) -> ConceptQuery {
        let descriptor = serde_json::from_value(json!({
            "with": { field: { "the": the, "as": kind, "cardinality": "one" } }
        }))
        .expect("descriptor parses");
        let mut terms = Parameters::new();
        terms.insert("this".to_string(), Term::var("this"));
        terms.insert(field.to_string(), Term::var(field));
        ConceptQuery {
            predicate: descriptor,
            terms,
        }
    }

    type Env =
        dialog_peer::Peer<dialog_storage::provider::storage::VolatileSpace, dialog_peer::Session>;

    async fn commit(branch: &Branch, operator: &Env, text: &str) -> anyhow::Result<Duration> {
        let parsed = parse(text);
        anyhow::ensure!(
            parsed.diagnostics.is_empty(),
            "document diagnostics: {:?}",
            parsed.diagnostics
        );
        let syntax = parsed.syntax.expect("syntax");
        let start = Instant::now();
        syntax
            .evaluate(branch.transaction())
            .perform(operator)
            .await
            .map_err(|error| anyhow::anyhow!("evaluate: {error}"))?
            .commit()
            .publish()
            .perform(operator)
            .await
            .map_err(|error| anyhow::anyhow!("commit: {error}"))?;
        Ok(start.elapsed())
    }

    async fn seed_library(
        branch: &Branch,
        operator: &Env,
        file: &str,
        text: &str,
    ) -> anyhow::Result<Duration> {
        let parsed = parse_at(tonk_library::location(file), text);
        anyhow::ensure!(
            parsed.diagnostics.is_empty(),
            "{file} diagnostics: {:?}",
            parsed.diagnostics
        );
        let mut syntax = parsed.syntax.expect("syntax");
        let unexpanded = expand(&mut syntax, &tonk_library::Bundled).await;
        anyhow::ensure!(unexpanded.is_empty(), "{file} includes: {unexpanded:?}");
        let start = Instant::now();
        syntax
            .evaluate(branch.transaction())
            .perform(operator)
            .await
            .map_err(|error| anyhow::anyhow!("evaluate {file}: {error}"))?
            .commit()
            .publish()
            .perform(operator)
            .await
            .map_err(|error| anyhow::anyhow!("commit {file}: {error}"))?;
        Ok(start.elapsed())
    }

    async fn rows(branch: &Branch, operator: &Env, query: &ConceptQuery) -> anyhow::Result<usize> {
        let rows = branch
            .select(QueryPlan::from(query.clone()))
            .perform(operator)
            .try_vec()
            .await
            .map_err(|error| anyhow::anyhow!("query: {error}"))?;
        Ok(rows.len())
    }

    /// Time `runs` evaluations of `query` after one warm-up, reporting the
    /// median and the row count.
    async fn time_query(
        label: &str,
        branch: &Branch,
        operator: &Env,
        query: &ConceptQuery,
        runs: usize,
    ) -> anyhow::Result<()> {
        let count = rows(branch, operator, query).await?;
        let mut samples = Vec::with_capacity(runs);
        for _ in 0..runs {
            let start = Instant::now();
            let again = rows(branch, operator, query).await?;
            samples.push(start.elapsed());
            anyhow::ensure!(again == count, "{label}: row count moved");
        }
        samples.sort();
        let median = samples[samples.len() / 2];
        let min = samples[0];
        println!(
            "{label:<28} rows={count:>6}  median={:>9.3} ms  min={:>9.3} ms",
            median.as_secs_f64() * 1e3,
            min.as_secs_f64() * 1e3
        );
        Ok(())
    }

    fn ms(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1e3
    }

    pub async fn run() -> anyhow::Result<()> {
        let accounts = size("ACCOUNTS", 2000);
        let spaces = size("SPACES", 500);
        let blocks = size("BLOCKS", 500);
        let runs = size("RUNS", 20);

        let (operator, profile) = test_session_with_peer().await;
        let repo = test_repo(&operator, &profile).await;
        let branch = repo.branch("main").open().perform(&operator).await?;

        println!("library seed");
        for (file, text) in [
            ("profile.yaml", PROFILE_LIBRARY),
            ("notebook.yaml", NOTEBOOK_LIBRARY),
        ] {
            let took = seed_library(&branch, &operator, file, text).await?;
            println!("  {file:<24} {:>9.1} ms", ms(took));
        }

        // The device profile the session reports: what a replica of a space
        // on this device is keyed by.
        let session = query("profile", "dialog.session/profile", "Entity");
        let sessions = branch
            .select(QueryPlan::from(session))
            .perform(&operator)
            .try_vec()
            .await
            .map_err(|error| anyhow::anyhow!("session: {error}"))?;
        let device = sessions
            .first()
            .and_then(|row| row.source().value_of("profile").cloned())
            .ok_or_else(|| anyhow::anyhow!("no session profile"))?;
        let device = match device {
            dialog_query::Value::Entity(entity) => entity.to_string(),
            other => anyhow::bail!("session profile is not an entity: {other:?}"),
        };

        // Accounts in the four cases, a quarter each: onboarding (minted,
        // not registered), registered only, registered and active, and
        // suspended on top of that.
        let mut doc = String::new();
        for i in 0..accounts {
            let this = format!("id:account-{i}");
            match i % 4 {
                0 => doc.push_str(&format!(
                    "account/onboarding!:\n  this: {this}\n  minted-at: {i}\n\n"
                )),
                _ => {
                    doc.push_str(&format!(
                    "account/registered!:\n  this: {this}\n  registered-at: {i}\n  email: \"account-{i}@example.com\"\n  provider: \"https://access.example/\"\n\n"
                ));
                    if i % 4 >= 2 {
                        doc.push_str(&format!(
                            "account/active!:\n  this: {this}\n  activated-at: {i}\n\n"
                        ));
                    }
                    if i % 4 == 3 {
                        doc.push_str(&format!(
                        "account/suspended!:\n  this: {this}\n  suspended-at: {i}\n  reason: \"load\"\n\n"
                    ));
                    }
                }
            }
        }
        // Spaces: remote (no replica), replicating, seeding (replica with a
        // blank status) and replicated, a quarter each.
        for i in 0..spaces {
            doc.push_str(&format!(
                "space!:\n  this: id:space-{i}\n  subject: id:subject-{i}\n\n"
            ));
            match i % 4 {
            0 => {}
            1 => doc.push_str(&format!(
                "space/replicating!:\n  this: id:replicating-{i}\n  subject: id:subject-{i}\n  replicating: true\n\n"
            )),
            2 => doc.push_str(&format!(
                "space/replica!:\n  this: id:replica-{i}\n  subject: id:subject-{i}\n  profile: {device}\n\nspace/replica-status!:\n  this: id:replica-{i}\n  status: tonk:blank\n\n"
            )),
            _ => doc.push_str(&format!(
                "space/replica!:\n  this: id:replica-{i}\n  subject: id:subject-{i}\n  profile: {device}\n\nspace/replica-status!:\n  this: id:replica-{i}\n  status: tonk:initialized\n\n"
            )),
        }
        }
        // One notebook holding every block under a fractional position key.
        doc.push_str("notebook!:\n  this: id:notebook\n  title: \"load\"\n  block:\n");
        for i in 0..blocks {
            doc.push_str(&format!("    N{i:06}: id:block-{i}\n"));
        }
        doc.push('\n');
        for i in 0..blocks {
            doc.push_str(&format!(
            "notebook/block!:\n  this: id:block-{i}\n  notebook: id:notebook\n  source: \"block {i}\"\n\n"
        ));
        }
        let took = commit(&branch, &operator, &doc).await?;
        println!(
            "data seed: {accounts} accounts, {spaces} spaces, {blocks} blocks in {:.1} ms",
            ms(took)
        );

        // `PROBE=1` installs probe rules of the presence shape one premise
        // at a time and counts their rows, to find which premise a
        // build loses.
        if std::env::var("PROBE").is_ok() {
            let probes: [(&str, &str); 7] = [
                ("probe0", ""),
                (
                    "probe6",
                    "    - assert: ==\n      where: {this: ?presence, is: case:remote}\n",
                ),
                (
                    "probe1",
                    "    - assert: ==\n      where: {this: ?presence, is: case:remote}\n",
                ),
                (
                    "probe2",
                    "    - assert: db/session\n      where: {profile: ?profile}\n    - assert: ==\n      where: {this: ?presence, is: case:remote}\n",
                ),
                (
                    "probe3",
                    "    - assert: ==\n      where: {this: ?presence, is: case:remote}\n  unless:\n    - assert: space/replicating\n      where: {subject: ?subject}\n",
                ),
                (
                    "probe4",
                    "    - assert: db/session\n      where: {profile: ?profile}\n    - assert: ==\n      where: {this: ?presence, is: case:remote}\n  unless:\n    - assert: space/replica\n      where: {subject: ?subject, profile: ?profile}\n",
                ),
                (
                    "probe5",
                    "    - assert: db/session\n      where: {profile: ?profile}\n    - assert: ==\n      where: {this: ?presence, is: case:remote}\n  unless:\n    - assert: space/replica\n      where: {subject: ?subject, profile: ?profile}\n    - assert: space/replicating\n      where: {subject: ?subject}\n",
                ),
            ];
            for (name, body) in probes {
                let lead = match name {
                    "probe0" => {
                        "    - assert: space\n      where: {this: ?this, subject: ?presence}\n"
                    }
                    "probe6" => "    - assert: account/registered\n      where: {this: ?this}\n",
                    _ => "    - assert: space\n      where: {this: ?this, subject: ?subject}\n",
                };
                let doc = format!(
                    "concept!: &{name}\n  this: tonk:{name}\n  description: \"a probe concept\"\n  with:\n    presence:\n      description: \"a probe field\"\n      the: xyz.tonk.{name}/presence\n      cardinality: one\n      as: entity\n\nrule!:\n  description: \"a probe rule\"\n  assert: {name}\n  when:\n{lead}{body}"
                );
                commit(&branch, &operator, &doc).await?;
                let probe = query("presence", &format!("xyz.tonk.{name}/presence"), "Entity");
                let count = rows(&branch, &operator, &probe).await?;
                println!("  {name:<8} rows={count}");
            }
            let spaces_query = query("subject", "xyz.tonk.space/subject", "Entity");
            println!(
                "  space subjects rows={}",
                rows(&branch, &operator, &spaces_query).await?
            );
            let session_query = query("profile", "dialog.session/profile", "Entity");
            println!(
                "  db/session rows={}",
                rows(&branch, &operator, &session_query).await?
            );
            return Ok(());
        }

        let status = query("status", "xyz.tonk.account/status", "Entity");
        let presence = query("presence", "xyz.tonk.space/presence", "Entity");
        let position = query("at", "xyz.tonk.notebook.block/at", "Text");
        let mut one = status.clone();
        one.terms.insert(
            "this".to_string(),
            Term::Constant(dialog_query::Value::Entity("id:account-7".parse()?)),
        );

        // `ONLY=status,presence,position,one` limits the timed queries,
        // for profiling one of them.
        let only = std::env::var("ONLY").unwrap_or_default();
        let wanted = |name: &str| only.is_empty() || only.split(',').any(|each| each == name);
        println!("queries (median of {runs})");
        if wanted("status") {
            time_query("account/status", &branch, &operator, &status, runs).await?;
        }
        if wanted("one") {
            time_query("account/status (one)", &branch, &operator, &one, runs).await?;
        }
        if wanted("presence") {
            time_query("space/presence", &branch, &operator, &presence, runs).await?;
        }
        if wanted("position") {
            time_query("block/position", &branch, &operator, &position, runs).await?;
        }
        if std::env::var("SKIP_SUBSCRIPTIONS").is_ok() {
            return Ok(());
        }

        println!("subscriptions");
        let mut statuses = branch.subscribe(QueryPlan::from(status.clone()));
        let mut presences = branch.subscribe(QueryPlan::from(presence.clone()));
        let start = Instant::now();
        let initial = statuses.poll(&operator).await?.expect("initial");
        println!(
            "  account/status initial     rows={:>6}  {:>9.3} ms",
            initial.asserted.len(),
            ms(start.elapsed())
        );
        let start = Instant::now();
        let initial = presences.poll(&operator).await?.expect("initial");
        println!(
            "  space/presence initial     rows={:>6}  {:>9.3} ms",
            initial.asserted.len(),
            ms(start.elapsed())
        );

        // Ten active accounts are suspended: each moves from case:active to
        // case:suspended, so the delta is ten retractions and ten assertions.
        let mut change = String::new();
        for i in (0..accounts).filter(|i| i % 4 == 2).take(10) {
            change.push_str(&format!(
            "account/suspended!:\n  this: id:account-{i}\n  suspended-at: 9\n  reason: \"load\"\n\n"
        ));
        }
        let took = commit(&branch, &operator, &change).await?;
        let start = Instant::now();
        let delta = statuses.poll(&operator).await?;
        let polled = start.elapsed();
        let (asserted, retracted) = delta
            .as_ref()
            .map(|delta| (delta.asserted.len(), delta.retracted.len()))
            .unwrap_or_default();
        println!(
            "  suspend 10: commit {:>8.1} ms, poll {:>9.3} ms, +{asserted} -{retracted}",
            ms(took),
            ms(polled)
        );
        anyhow::ensure!(asserted == 10 && retracted == 10, "status delta is wrong");
        let start = Instant::now();
        let unchanged = presences.poll(&operator).await?;
        println!(
            "  unrelated poll             {:>9.3} ms, changed={}",
            ms(start.elapsed()),
            unchanged
                .is_some_and(|delta| !delta.asserted.is_empty() || !delta.retracted.is_empty())
        );

        // Ten seeding replicas finish: each space moves from case:seeding
        // to case:replicated.
        let mut change = String::new();
        for i in (0..spaces).filter(|i| i % 4 == 2).take(10) {
            change.push_str(&format!(
                "space/replica-status!:\n  this: id:replica-{i}\n  status: tonk:initialized\n\n"
            ));
        }
        let took = commit(&branch, &operator, &change).await?;
        let start = Instant::now();
        let delta = presences.poll(&operator).await?;
        let polled = start.elapsed();
        let (asserted, retracted) = delta
            .as_ref()
            .map(|delta| (delta.asserted.len(), delta.retracted.len()))
            .unwrap_or_default();
        println!(
            "  replicate 10: commit {:>6.1} ms, poll {:>9.3} ms, +{asserted} -{retracted}",
            ms(took),
            ms(polled)
        );
        anyhow::ensure!(asserted == 10 && retracted == 10, "presence delta is wrong");
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    load::run().await
}

#[cfg(target_arch = "wasm32")]
fn main() {}
