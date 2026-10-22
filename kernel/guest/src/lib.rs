//! A Tonk guest kernel: a component for the world `sapling:kernel`, which the
//! same host and bootloader run as any other kernel. It opens the space's
//! repository of Dialog DB on the layer of `memory:space`, signs through
//! `session`, and serves the space's data over `wasi:http/handler`.
//!
//! Routes:
//! - `GET /api/health`: `ok` once booted.
//! - `POST /api/transact`: a JSON list of `{"the", "of", "is"}` (text values),
//!   asserted on `main`. Answers who issued the new head and whom it names as
//!   its authority.
//! - `POST /api/select`: `{"the"}`, answers the matching facts as JSON.
//! - `POST /api/credentials/check`: generates a credential of the space, signs
//!   a UCAN invocation with it through the host, verifies it, and opens a
//!   secret sealed to it. Answers what each step found.

pub mod world {
    wit_bindgen::generate!({
        path: "wit",
        world: "sapling:kernel/kernel",
        generate_all,
    });
}

mod cache;
mod credentials;
mod guest;
mod http;
mod layer;
mod mount;
mod repository;
mod signal;
mod store;

use std::sync::{Mutex, PoisonError};

use dialog_artifacts::{Artifact, ArtifactSelector, Instruction, Value};
use serde_json::{Value as Json, json};

use repository::Repository;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::sapling::kernel::types::Error;
use world::wasi::http::types::{ErrorCode, Method, Request, Response};

/// The space's repository, once `init` opened it. A `static`, not a
/// thread-local: on `wasm32-wasip3` each export call is a task of its own,
/// with thread-locals of its own.
static REPOSITORY: Mutex<Held> = Mutex::new(Held(None));

/// The DID of the space, once `init` opened it.
static SPACE: Mutex<Option<String>> = Mutex::new(None);

/// Dialog DB's peer is not `Send`; a component runs on one thread, so a
/// `static` may hold it.
struct Held(Option<Repository>);

// SAFETY: a component instance runs on a single thread; nothing here is
// reached from another.
unsafe impl Send for Held {}

fn repository() -> Option<Repository> {
    REPOSITORY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .0
        .clone()
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

struct Kernel;

impl lifecycle::Guest for Kernel {
    async fn init(boot: lifecycle::Boot) -> Result<(), Error> {
        let started = world::wasi::clocks::monotonic_clock::now();
        let space = boot
            .stack
            .into_iter()
            .find(|stacked| stacked.scopes.iter().any(|scope| scope == "memory:space"))
            .ok_or_else(|| invalid("the boot binds no layer under memory:space"))?;
        let layer = guest::HandleLayer::new(space.layer).map_err(invalid)?;
        let subject = layer::Layer::subject(&layer);
        *SPACE.lock().unwrap_or_else(PoisonError::into_inner) = Some(subject.to_string());
        let signer = guest::session_signer().map_err(invalid)?;
        // A worker that holds a space for a persona other than the space
        // writes as that persona.
        let profile = boot
            .persona
            .filter(|persona| *persona != subject.to_string())
            .map(|persona| persona.parse().map_err(invalid))
            .transpose()?;
        let opened = Repository::open(layer::share(layer), signer, profile)
            .await
            .map_err(invalid)?;
        REPOSITORY.lock().unwrap_or_else(PoisonError::into_inner).0 = Some(opened);
        let took = world::wasi::clocks::monotonic_clock::now() - started;
        eprintln!("tonk-kernel: opened the space in {}us", took / 1000);
        Ok(())
    }

    async fn sync(_changes: Vec<lifecycle::CellChange>) -> Result<(), Error> {
        Ok(())
    }

    async fn shutdown(_deadline_ms: u32) {}
}

impl channel::Guest for Kernel {
    async fn receive(_catchup: Vec<u8>) -> Result<(), Error> {
        Ok(())
    }
}

fn text(json: &Json, field: &str) -> Result<String, String> {
    json.get(field)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("`{field}` is not text"))
}

async fn transact(body: &[u8]) -> Result<Json, String> {
    let repository = repository().ok_or("the kernel has not booted")?;
    let claims: Vec<Json> = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let instructions = claims
        .iter()
        .map(|claim| {
            Ok(Instruction::Assert(
                Artifact {
                    the: text(claim, "the")?
                        .parse()
                        .map_err(|error| format!("{error}"))?,
                    of: text(claim, "of")?
                        .parse()
                        .map_err(|error| format!("{error}"))?,
                    is: Value::String(text(claim, "is")?),
                    cause: None,
                },
                dialog_artifacts::Pick::All,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let branch = repository
        .branch("main")
        .await
        .map_err(|error| error.to_string())?;
    branch
        .commit(instructions)
        .await
        .map_err(|error| error.to_string())?;
    let branch = repository
        .branch("main")
        .await
        .map_err(|error| error.to_string())?;
    let attribution = branch
        .attribution()
        .await
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "committed": claims.len(),
        "issuer": attribution.as_ref().map(|(issuer, _)| issuer),
        "authority": attribution.as_ref().map(|(_, authority)| authority),
    }))
}

async fn check_credentials() -> Result<Json, String> {
    use dialog_credentials::Ed25519Verifier;
    use dialog_credentials::secret::Context;
    use dialog_ucan_core::InvocationBuilder;
    use world::sapling::kernel::{credentials as host, session, storage};

    const CHECK: Context = Context::new("tonk/kernel/check/v1");
    let space = SPACE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or("the kernel has not booted")?;
    let layer = storage::open(space.clone(), session::chain())
        .await
        .map_err(|error| format!("{error:?}"))?;
    let credential = host::generate(&layer, "check".to_owned())
        .await
        .map_err(|error| format!("{error:?}"))?;
    let did = credential.did();
    let verifier: Ed25519Verifier = did.parse().map_err(|_| "not an Ed25519 did:key")?;

    let sealed = verifier
        .secret(CHECK)
        .conceal(b"a vault key")
        .await
        .map_err(|error| error.to_string())?;
    let revealed = credential
        .reveal(CHECK.as_str().to_owned(), sealed.to_bytes())
        .await
        .map_err(|error| format!("{error:?}"))?;

    let signer = credentials::signer(credential).map_err(|error| error.to_string())?;
    let subject: dialog_varsig::Did = space.parse().map_err(|_| "the space is not a DID")?;
    let invocation = InvocationBuilder::new()
        .issuer(signer)
        .audience(&subject)
        .subject(&subject)
        .command(vec!["storage".to_owned(), "get".to_owned()])
        .proofs(Vec::new())
        .try_build()
        .await
        .map_err(|error| error.to_string())?;
    let verified = invocation
        .verify_with::<std::convert::Infallible>(&dialog_credentials::Verifier::from(verifier))
        .await;
    Ok(json!({
        "did": did,
        "invocation": verified.is_ok(),
        "revealed": revealed == b"a vault key",
    }))
}

async fn select(body: &[u8]) -> Result<Json, String> {
    let repository = repository().ok_or("the kernel has not booted")?;
    let query: Json = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let the = text(&query, "the")?
        .parse()
        .map_err(|error| format!("{error}"))?;
    let branch = repository
        .branch("main")
        .await
        .map_err(|error| error.to_string())?;
    let facts = branch
        .select(ArtifactSelector::new().the(the))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Json::Array(
        facts
            .into_iter()
            .map(|fact| json!({ "the": fact.the.to_string(), "of": fact.of.to_string(), "is": format!("{:?}", fact.is) }))
            .collect(),
    ))
}

impl handler::Guest for Kernel {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let (method, path, _) = http::target(&request);
        let answer = match (&method, path.as_str()) {
            (Method::Get, "/api/health") => {
                return Ok(http::respond(
                    200,
                    Some("text/plain"),
                    match repository() {
                        Some(_) => b"ok".to_vec(),
                        None => b"booting".to_vec(),
                    },
                ));
            }
            (Method::Post, "/api/transact") => transact(&http::read_body(request).await?).await,
            (Method::Post, "/api/select") => select(&http::read_body(request).await?).await,
            (Method::Post, "/api/credentials/check") => check_credentials().await,
            _ => return Ok(http::respond(404, None, b"not found".to_vec())),
        };
        Ok(match answer {
            Ok(body) => http::json(200, body.to_string()),
            Err(error) => http::json(500, json!({ "error": error }).to_string()),
        })
    }
}

world::export!(Kernel with_types_in world);
