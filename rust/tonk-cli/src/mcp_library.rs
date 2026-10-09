//! Bounded first installation of bundled note libraries for native MCP hosts.

use anyhow::{Result, anyhow, bail};
use base58::ToBase58 as _;
use dialog_artifacts::{Changes, Entity, Statement as _};
use dialog_query::{Output as _, Query, Term};
use serde_json::{Value, json};
use tonk_schema::{SeedAvailable, SeedInstall, domain::seed};

use crate::site::TonkSite;

async fn parse(text: &str) -> Result<tonk_notation::Syntax> {
    let parsed = tonk_notation::parse_at(tonk_library::location("core.yaml"), text);
    if !parsed.diagnostics.is_empty() {
        bail!("Bundled library failed to parse.");
    }
    let mut syntax = parsed.syntax.ok_or_else(|| anyhow!("Empty library."))?;
    if !tonk_notation::expand(&mut syntax, &tonk_library::Bundled)
        .await
        .is_empty()
    {
        bail!("Bundled library includes failed to expand.");
    }
    Ok(syntax)
}

pub(crate) async fn install(site: &TonkSite, arguments: &Value) -> Result<Value> {
    let fields = arguments
        .as_object()
        .ok_or_else(|| anyhow!("Expected an object."))?;
    if fields
        .keys()
        .any(|key| key != "component" && key != "expectedRevision")
    {
        bail!("Provide only component and optional expectedRevision.");
    }
    let component = fields
        .get("component")
        .and_then(Value::as_str)
        .unwrap_or("");
    let library = match component {
        "prose" => include_str!("../../tonk-core/assets/library/prose.yaml"),
        "notebook" => include_str!("../../tonk-core/assets/library/notebook.yaml"),
        _ => bail!("Only bundled prose and notebook libraries are supported."),
    };
    let source = format!("/library/{component}.yaml");
    let seed: Entity = format!("seed:{}", blake3::hash(library.as_bytes()).to_hex()).parse()?;
    let mut syntax = parse(include_str!("../../tonk-core/assets/library/core.yaml")).await?;
    let first = syntax.expressions.len();
    syntax.expressions.extend(parse(library).await?.expressions);
    let (durable, commands) =
        tonk_evaluator::library::plan_install(&syntax, first).map_err(anyhow::Error::msg)?;
    // These two bundled components contain declarations but dispatch no commands.
    // Fail closed if a later library revision introduces post-commit side effects.
    if !commands.into_instructions().is_empty() {
        bail!("This bundled library requires commands unsupported by this host.");
    }
    let claims = durable.clone().into_instructions().len();
    let session = site.branch().await?;
    let _committing = session.transactor().lock().await;
    let before = session.handle().revision();
    let applying = fields.contains_key("expectedRevision");
    if applying {
        let expected = &fields["expectedRevision"];
        if serde_json::to_vec(expected)?.len() > 16_000 {
            bail!("Invalid expectedRevision.");
        }
        let expected = serde_json::from_value(expected.clone())?;
        if before != expected {
            bail!(
                "The space changed since preview. Nothing was installed. Read and preview again."
            );
        }
    }
    let available = session
        .handle()
        .query()
        .select(Query::<SeedAvailable> {
            this: Term::var("seed"),
            source: Term::from(seed::Source(source.clone())),
            replaces: Term::var("prior"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    let installed = session
        .handle()
        .query()
        .select(Query::<SeedInstall> {
            this: Term::from(seed.clone()),
            prior: Term::var("prior"),
            version: Term::var("version"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    if !installed.is_empty() && available.iter().any(|record| record.this == seed) {
        return Ok(
            json!({"component": component, "revision": before, "committed": false,
            "alreadyInstalled": true, "scope": "This bundled version is already installed; no write was performed."}),
        );
    }
    if !available.is_empty() {
        bail!(
            "This component already has library provenance. Upgrades and repairs must use Tonk's existing library manager; nothing was installed."
        );
    }
    // Do not take ownership of an existing, untracked model with this name.
    use dialog_query::{AttributeQuery, attribute};
    let names = session
        .handle()
        .query()
        .select(AttributeQuery::new(
            Term::from(attribute::The::from(
                "db.name/referent".parse::<dialog_artifacts::Attribute>()?,
            )),
            Term::from(format!("id:{component}").parse::<Entity>()?),
            Term::<dialog_query::Any>::var("target"),
            Term::<attribute::Cause>::blank(),
            None,
        ))
        .perform(&site.operator)
        .try_vec()
        .await?;
    if !names.is_empty() {
        bail!(
            "A model with this name already exists without matching install provenance. Nothing was installed."
        );
    }
    if !applying {
        return Ok(
            json!({"component": component, "source": source, "seed": seed,
            "revision": before, "committed": false, "claims": claims,
            "scope": "Preview of the bundled library manifest only. No installation or rendered preview. Apply with this expectedRevision."}),
        );
    }
    // Match the browser's complete-install protocol: stage the entire manifest,
    // then its provenance, and publish both at once. Never retry an uncertain write.
    let result: Result<_> = async {
        let installed = session
            .handle()
            .transaction()
            .assert(durable)
            .commit()
            .perform(&site.operator)
            .await?;
        let mut record = Changes::new();
        let prior: Entity = "seed:none".parse()?;
        SeedAvailable {
            this: seed.clone(),
            source: seed::Source(source),
            replaces: seed::Replaces(prior.clone()),
        }
        .assert(&mut record);
        SeedInstall {
            this: seed.clone(),
            prior: seed::Prior(prior),
            version: seed::InstallVersion(installed.version().key_bytes().to_base58()),
        }
        .assert(&mut record);
        let revision = installed
            .transaction()
            .assert(record)
            .commit()
            .perform(&site.operator)
            .await?
            .publish()
            .perform(&site.operator)
            .await?;
        Ok(revision)
    }
    .await;
    let revision = result.map_err(|_| anyhow!("Library installation outcome could not be confirmed. Query the space before deciding what to do; do not repeat the install automatically."))?;
    session.poll(&site.operator).await;
    Ok(
        json!({"component": component, "seed": seed, "revision": revision,
        "previousRevision": before, "committed": true, "claims": claims,
        "renderingConfirmed": false, "scope": "Library and provenance committed locally. Remote synchronization and rendering are not confirmed."}),
    )
}
