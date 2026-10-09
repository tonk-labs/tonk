//! Complete library manifests, shared by native hosts and the browser installer.

use dialog_artifacts::{Changes, Statement as _};
use dialog_query::{Parameters, Term};
use tonk_schema::transact::{ApplicationPlan, Planner as _, Statement};

/// Lower only expressions from `first`, resolving against the entire document.
/// Analyze without a branch: existing claims must not suppress install provenance.
/// The returned changes separate durable assertions from transient commands.
pub fn plan_install(
    syntax: &tonk_notation::Syntax,
    first: usize,
) -> Result<(Changes, Changes), String> {
    let analyzed = tonk_analyzer::analyzer::analyze_local(syntax)
        .map_err(|error| format!("analyze library: {error}"))?;
    let mut bindings = Parameters::new();
    for (name, entity) in &analyzed.analysis.variables {
        bindings.insert(
            name.clone(),
            Term::Constant(dialog_artifacts::Value::Entity(entity.clone())),
        );
    }

    let transient = analyzed.analysis.transient_entities();
    let mut desired = Changes::new();
    let mut commands = Changes::new();
    for planned in analyzed.analysis.statements_from(first) {
        match planned.statement {
            Statement::Assert(application) => {
                let plan = application
                    .plan(&bindings)
                    .map_err(|error| format!("plan library: {error}"))?;
                let command = matches!(
                    &plan,
                    ApplicationPlan::Concept(concept)
                        if transient.contains(&concept.statement.predicate.this())
                );
                if command {
                    plan.assert(&mut commands);
                } else {
                    plan.assert(&mut desired);
                }
            }
            Statement::Retract(_) => {
                return Err("Library desired manifest contains a retraction".into());
            }
        }
    }

    Ok((desired, commands))
}
