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

use crate::router::CommandEnv;
use crate::router::claim::RawClaim;

/// The attribute that ties an intent to its expression.
const INTENT_EXPRESSION: &str = "tonk.dialog.intent/expression";

fn claim(the: &str, of: &Entity, is: Value) -> Option<RawClaim> {
    Some(RawClaim {
        the: the.parse().ok()?,
        of: of.clone(),
        is,
        unique: true,
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
    async fn execute(&self, command: tonk_schema::command::Interpret) {
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
                return;
            }
        };
        let site_text = site.to_string();
        let interpretation = match dialog_reactor::interpret(
            &session.state,
            &tonk.operator,
            &input,
            Some(site_text.as_str()),
        )
        .await
        {
            Ok(interpretation) => interpretation,
            Err(error) => {
                log!("intent/interpret: {error}");
                return;
            }
        };

        // Replace the expression's previous intents: drop every entity that
        // points at it, and the expression itself, then write afresh.
        let prior: BTreeSet<Entity> = session
            .state
            .state_layer()
            .export()
            .iter()
            .filter(|(_, attribute, change)| {
                attribute.to_string() == INTENT_EXPRESSION
                    && matches!(
                        change,
                        dialog_artifacts::Change::Assert(Value::Entity(of))
                            | dialog_artifacts::Change::Replace(Value::Entity(of))
                            if *of == expression
                    )
            })
            .map(|(entity, _, _)| entity.clone())
            .collect();
        // The forgets ride the same commit as the new facts, so no reader
        // sees the expression between its old intents and its new ones.
        let mut overlay = branch().overlay().forget(expression.clone());
        for entity in prior {
            overlay = overlay.forget(entity);
        }
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
        }
    }
}
