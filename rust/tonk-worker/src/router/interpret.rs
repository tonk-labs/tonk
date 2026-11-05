//! `intent/interpret`: the command palette's parser, as a command handler.
//!
//! The palette sends what is typed, with the palette opening it belongs to
//! (the expression) and the tab's site. The handler parses it against the
//! commands the branch declares and records, in that branch's session
//! overlay:
//!
//! - the expression: its input, site and time, and the site's selection;
//! - one `intent` per command the input could mean, pointing at the
//!   expression and the command.
//!
//! Rules derive values for each command's fields onto its intent (the
//! notebook a page shows, for a command that takes a notebook), and
//! `intent/suggest` reads the intents and those values back as readings.
//! Each interpretation replaces the expression's previous intents. Session
//! facts only: nothing is stored, and it goes with the tab.

use std::collections::BTreeSet;

use dialog_artifacts::{Entity, Value};
use tonk_common::log;
use tonk_schema::invocation::{Done, Failure, Outcome};

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// The attribute that ties an intent to its expression.
const INTENT_EXPRESSION: &str = "tonk.dialog.intent/expression";

fn claim(the: &str, of: &Entity, is: Value) -> Option<RawClaim> {
    Some(RawClaim {
        the: the.parse().ok()?,
        of: of.clone(),
        is,
        policy: dialog_artifacts::Pick::Last,
    })
}

/// The intent for `command` in `expression`: the same entity each time the
/// expression is interpreted, so a reading of the same command keeps its
/// identity as the line changes.
fn intent(expression: &Entity, command: &str) -> Option<Entity> {
    let digest = blake3::hash(format!("{expression}\u{1f}{command}").as_bytes());
    format!("intent:{}", digest.to_hex()).parse().ok()
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::Interpret> for CommandEnv {
    async fn execute(&self, command: tonk_schema::command::Interpret) -> Outcome {
        let expression = command.expression.0;
        let input = command.input.0;
        let site = command.site.0;
        let time = command.time.0;
        let tonk = self.state().read().await;
        let origin = self.origin().clone();
        let branch = || {
            if origin.repo.is_empty() {
                tonk.reactor.profile_repository().branch(&origin.branch)
            } else {
                tonk.reactor.repository(&origin.repo).branch(&origin.branch)
            }
        };
        let session = match branch().acquire(&tonk.operator).await {
            Ok(session) => session,
            Err(error) => {
                log!("intent/interpret: branch unavailable: {error}");
                return Err(Failure::new(format!("the branch is unavailable: {error}")));
            }
        };
        let site_text = site.to_string();
        let interpretation = match dialog_reactor::interpret(
            session.handle(),
            &tonk.operator,
            &input,
            Some(site_text.as_str()),
        )
        .await
        {
            Ok(interpretation) => interpretation,
            Err(error) => {
                log!("intent/interpret: {error}");
                return Err(Failure::new(format!(
                    "the input could not be interpreted: {error}"
                )));
            }
        };

        // Replace the expression's previous intents: drop every entity that
        // points at it, and the expression itself, then write afresh.
        let prior: BTreeSet<Entity> = session
            .handle()
            .overlay()
            .export()
            .iter()
            .filter(|(_, attribute, change)| {
                attribute.to_string() == INTENT_EXPRESSION
                    && matches!(
                        change,
                        dialog_artifacts::Change::Assert(Value::Entity(of), _)
                            if *of == expression
                    )
            })
            .map(|(entity, _, _)| entity.clone())
            .collect();
        session
            .state
            .retain_overlay_entities(|entity| *entity != expression && !prior.contains(entity));

        let mut overlay = branch().overlay();
        let mut facts = vec![
            claim(
                "tonk.dialog.intent.expression/site",
                &expression,
                Value::Entity(site.clone()),
            ),
            claim(
                "tonk.dialog.intent.expression/time",
                &expression,
                Value::Float(time),
            ),
            interpretation.selection.and_then(|selection| {
                claim(
                    "tonk.dialog.intent.expression/selection",
                    &expression,
                    Value::String(selection),
                )
            }),
        ];
        for command in &interpretation.commands {
            let (Some(intent), Ok(command)) =
                (intent(&expression, command), command.parse::<Entity>())
            else {
                continue;
            };
            facts.push(claim(
                INTENT_EXPRESSION,
                &intent,
                Value::Entity(expression.clone()),
            ));
            facts.push(claim(
                "tonk.dialog.intent/command",
                &intent,
                Value::Entity(command),
            ));
        }
        // The input last: a reader waiting for this input sees the intents
        // with it, since the whole write lands at once.
        facts.push(claim(
            "tonk.dialog.intent.expression/input",
            &expression,
            Value::String(input),
        ));
        for fact in facts.into_iter().flatten() {
            overlay = overlay.assert(fact);
        }
        if let Err(error) = overlay.write().perform(&tonk.operator).await {
            log!("intent/interpret: overlay write failed: {error}");
            return Err(Failure::new(format!(
                "the interpretation could not be recorded: {error}"
            )));
        }
        Done::with(expression)
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use crate::router::command::tests::native::test_state;
    use crate::router::{CommandEnv, CommandOrigin};
    use tonk_schema::domain::command::current::intent_interpret::{Expression, Input, Site, Time};

    /// Input typed in a space this worker does not have cannot be
    /// interpreted, and the outcome says the branch is unavailable instead
    /// of the palette waiting on intents that never come.
    #[dialog_common::test]
    async fn it_fails_to_interpret_in_a_missing_repository() {
        let state = test_state().await;
        let env = CommandEnv::new(
            state,
            CommandOrigin {
                repo: "did:key:z6MkMissingRepositoryForInterpretTest".into(),
                branch: "main".into(),
                client: None,
            },
        );
        let command = tonk_schema::command::Interpret {
            this: "id:interpret".parse().unwrap(),
            expression: Expression("id:expression".parse().unwrap()),
            input: Input("rename".into()),
            site: Site("site:test".parse().unwrap()),
            time: Time(0.0),
        };
        let outcome =
            dialog_capability::Provider::<tonk_schema::command::Interpret>::execute(&env, command)
                .await;
        assert!(
            matches!(&outcome, Err(failure) if failure.0.contains("unavailable")),
            "{outcome:?}"
        );
    }
}
