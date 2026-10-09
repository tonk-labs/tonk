//! Read-only bridge for the existing Tonk guest renderer. Routing context is
//! ephemeral; requests cannot select another repository or commit claims.
use crate::site::TonkSite;
use anyhow::{Result, bail, ensure};
use dialog_artifacts::{Attribute, Entity, Statement, Update, Value};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::schema::{
    DidExt as _, Replica,
    replica::{Peer, Subject},
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    path: String,
    #[serde(default)]
    query: Option<tonk_schema::query::Query>,
}
struct ContextClaim(Attribute, Entity, Value);
impl Statement for ContextClaim {
    fn assert(self, update: &mut impl Update) {
        update.associate_unique(self.0, self.1, self.2);
    }
    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.0, self.1, self.2);
    }
}

pub async fn read(site: &TonkSite, input: Json) -> Result<Json> {
    ensure!(
        serde_json::to_vec(&input)?.len() <= 100_000,
        "UI request is too large."
    );
    let input: Request = serde_json::from_value(input)?;
    ensure!(
        input.path.starts_with('/')
            && !input.path.starts_with("//")
            && input.path.len() <= 4096
            && !input.path.contains(['?', '#', '\\'])
            && !input.path.chars().any(char::is_control),
        "Invalid space path."
    );
    let branch = site.reactor.repository("main").branch("main");
    let session = site.branch().await?;
    let replicas: Vec<Replica> = session
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::from(Subject(site.repository.did().this())),
            peer: Term::from(Peer(site.profile.did().this())),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    let replica = replicas
        .first()
        .ok_or_else(|| anyhow::anyhow!("Replica context unavailable."))?
        .this
        .clone();
    let mut routes: Vec<tonk_schema::Route> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::Route> {
            this: Term::var("this"),
            path: Term::var("path"),
            concept: Term::var("concept"),
        })
        .perform(&site.operator)
        .try_vec()
        .await?;
    routes.sort_by(|a, b| a.this.to_string().cmp(&b.this.to_string()));
    let mut router = tonk_router::Router::new();
    for route in routes {
        if let Ok(pattern) = tonk_router::Route::parse_pattern(&route.path.0) {
            router.insert(pattern, (route.this, route.concept.0));
        }
    }
    let matched = router
        .recognize(&input.path)
        .map_err(|_| anyhow::anyhow!("No space route matches this path."))?;
    let (route, concept) = matched.value;
    let entity: Entity = "site:chatgpt".parse()?;
    session
        .state
        .retain_overlay_entities(|existing| existing != &entity);
    let subject = site.repository.did().to_string();
    let stamp = tonk_schema::Site::new(
        entity.clone(),
        input.path.clone(),
        String::new(),
        subject.clone(),
        "main".into(),
        replica,
        route.clone(),
        concept.clone(),
        "main".into(),
    );
    let mut overlay = branch.overlay().assert(stamp);
    for (name, value) in matched
        .params
        .iter()
        .chain([("repo", subject.as_str()), ("branch", "main")])
    {
        let value = percent_encoding::percent_decode_str(value).decode_utf8()?;
        let value = if name == "entity" {
            Value::Entity(value.parse()?)
        } else {
            Value::String(value.into_owned())
        };
        overlay = overlay.assert(ContextClaim(
            format!("xyz.tonk.site/{name}").parse()?,
            entity.clone(),
            value,
        ));
    }
    overlay.write().perform(&site.operator).await?;
    let rows = if let Some(query) = input.query {
        let Ok(query) = query.into_concept_query() else {
            bail!("Introspection formulas are not supported in this read-only embed.");
        };
        let rows = branch.query(query).perform(&site.operator).await?;
        ensure!(
            serde_json::to_vec(&rows)?.len() <= 500_000,
            "UI query result is too large."
        );
        Some(rows)
    } else {
        None
    };
    Ok(
        json!({"subject": subject, "path": input.path, "site": entity.to_string(), "model": concept.to_string(), "rows": rows}),
    )
}
