//! Public `tonk link` adapter. Local and headless terminals use the same signed polling.

use super::*;
use std::{
    io::Write as _,
    time::{Duration, Instant},
};
use tonk_invite::terminal::{Addition, ReadAdditions};
use tonk_invite::terminal::{MAX_APPROVAL_BYTES, ReadRequest};

/// Explicit public command options; no ambient CLI account participates.
#[derive(Debug, Default)]
pub struct LinkOptions {
    /// Explicitly replace active account-wide remote use with separate scoped replicas.
    pub convert_account: bool,
    /// Explicit deployment origin, otherwise the accountless deployment default.
    pub via: Option<String>,
    /// Print the public approval URL without opening a browser.
    pub no_open: bool,
    /// User-visible terminal label.
    pub label: Option<String>,
    /// Explicit account constraint, never inferred from current CLI login.
    pub expected_account: Option<String>,
    /// Resume this exact retained request without generating another key.
    pub resume: Option<String>,
    /// Cancel this local request, without promising remote withdrawal.
    pub cancel: Option<String>,
    /// Local waiting limit, bounded by the signed approval window.
    pub timeout_seconds: Option<u64>,
}

struct Deployment {
    origin: Url,
    service: Did,
    remote: Url,
    client: reqwest::Client,
}
async fn bounded_response(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= max as u64),
        "terminal delivery exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("terminal delivery read failed")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= max,
            "terminal delivery exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn discover(origin: Url) -> Result<Deployment> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()?;
    let response = client
        .get(origin.join(".well-known/tonk")?)
        .send()
        .await
        .context("terminal deployment discovery unavailable")?;
    ensure!(
        response.status().is_success(),
        "terminal deployment discovery failed"
    );
    let config: tonk_worker_api::DeploymentConfig =
        serde_json::from_slice(&bounded_response(response, 64 * 1024).await?)
            .context("terminal deployment configuration invalid")?;
    let service: Did = config
        .service_did
        .context("terminal deployment has no configured service identity")?
        .parse()
        .context("terminal deployment service identity invalid")?;
    Ok(Deployment {
        remote: origin.join("ucan/")?,
        origin,
        service,
        client,
    })
}
fn origin(value: &str) -> Result<Url> {
    let parsed = Url::parse(value).context("invalid terminal deployment origin")?;
    ensure!(
        parsed.query().is_none() && parsed.fragment().is_none(),
        "terminal deployment origin has a query or fragment"
    );
    crate::deployment::ceremony_origin(value)
}
fn trusted(approval: &Approval, deployment: &Deployment) -> Result<BTreeMap<String, Url>> {
    ensure!(
        approval.request().service() == &deployment.service,
        "terminal service identity changed"
    );
    let mut remotes = BTreeMap::new();
    for bundle in approval.bundles() {
        ensure!(
            bundle.remote() == &deployment.remote,
            "terminal selected space uses an untrusted service route"
        );
        remotes.insert(bundle.subject().to_string(), deployment.remote.clone());
    }
    Ok(remotes)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdditionsPage {
    deliveries: Vec<AdditionItem>,
    next_cursor: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdditionItem {
    sequence: u64,
    bytes: String,
}
fn trusted_addition(addition: &Addition, deployment: &Deployment) -> Result<BTreeMap<String, Url>> {
    ensure!(
        addition.service() == &deployment.service,
        "terminal addition service identity changed"
    );
    let mut remotes = BTreeMap::new();
    for bundle in addition.bundles() {
        ensure!(
            bundle.remote() == &deployment.remote,
            "terminal added space uses an untrusted service route"
        );
        remotes.insert(bundle.subject().to_string(), deployment.remote.clone());
    }
    Ok(remotes)
}
async fn additions(link: &TerminalLink, deployment: &Deployment) -> Result<Vec<TerminalSpace>> {
    let now = || dialog_ucan_core::time::Timestamp::now().to_unix();
    if let Some((sequence, bytes)) = link.pending_addition()? {
        consume_addition(link, deployment, sequence, &bytes, now()).await?;
    }
    loop {
        let cursor = link.cursor()?;
        let request = ReadAdditions::sign(
            &link.signer().await?,
            &link.request.id(),
            cursor,
            rand::random(),
            now(),
        )
        .await?;
        let response = deployment
            .client
            .post(deployment.origin.join("connection/additions/read")?)
            .header(reqwest::header::CONTENT_TYPE, "application/cbor")
            .body(request.bytes().to_vec())
            .send()
            .await
            .context("terminal additions polling interrupted")?;
        ensure!(
            response.status().is_success(),
            "terminal additions polling was refused"
        );
        let page: AdditionsPage = serde_json::from_slice(
            &bounded_response(response, MAX_APPROVAL_BYTES * 2 + 65536).await?,
        )
        .context("terminal additions response malformed")?;
        ensure!(
            page.deliveries.len() <= 1,
            "terminal additions page exceeds delivery bound"
        );
        if page.deliveries.is_empty() {
            ensure!(
                page.next_cursor == cursor,
                "terminal delivery cursor advanced without a delivery"
            );
            break;
        }
        let item = &page.deliveries[0];
        ensure!(
            item.sequence > cursor && page.next_cursor == item.sequence,
            "terminal delivery cursor mismatch"
        );
        let bytes = hex::decode(&item.bytes).context("terminal addition encoding invalid")?;
        consume_addition(link, deployment, item.sequence, &bytes, now()).await?;
    }
    for (sequence, id, reason) in link.rejected_deliveries()? {
        eprintln!(
            "Terminal delivery {sequence} ({id}) was not installed: {}. Any staged local data is retained; later deliveries can continue.",
            reason.as_str()
        );
    }
    link.installed_spaces()
}

async fn consume_addition(
    link: &TerminalLink,
    deployment: &Deployment,
    sequence: u64,
    bytes: &[u8],
    now: u64,
) -> Result<()> {
    let addition = Addition::inspect(bytes).await?;
    let routes = trusted_addition(&addition, deployment)?;
    link.resolve_addition(sequence, bytes, &routes, now, true)
        .await
}

async fn wait(
    link: &TerminalLink,
    deployment: &Deployment,
    timeout: Duration,
) -> Result<Vec<TerminalSpace>> {
    let now = || dialog_ucan_core::time::Timestamp::now().to_unix();
    let saved = journal(&link.root)?;
    if saved.state == LinkState::Completed {
        return additions(link, deployment).await;
    }
    if let Some(approval) = saved.approval {
        let approval = current_approval(&hex::decode(approval)?, now()).await?;
        return link
            .finish_and_sync(&trusted(&approval, deployment)?, now())
            .await;
    }
    let start = Instant::now();
    loop {
        let read = ReadRequest::sign(
            &link.signer().await?,
            &link.request.id(),
            rand::random(),
            now(),
        )
        .await?;
        let response = deployment
            .client
            .post(deployment.origin.join("connection/read")?)
            .header(reqwest::header::CONTENT_TYPE, "application/cbor")
            .body(read.bytes().to_vec())
            .send()
            .await
            .context("terminal approval polling interrupted")?;
        match response.status().as_u16() {
            200 => {
                let bytes = bounded_response(response, MAX_APPROVAL_BYTES).await?;
                let approval = current_approval(&bytes, now()).await?;
                ensure!(
                    approval.request().bytes() == link.request.bytes(),
                    "terminal mailbox returned another request"
                );
                let remotes = trusted(&approval, deployment)?;
                return link.accept_and_sync(&bytes, &remotes, now()).await;
            }
            204 => {}
            _ => anyhow::bail!("terminal approval polling was refused"),
        }
        if start.elapsed() >= timeout || now() >= link.request.deadline() {
            link.expire()?;
            anyhow::bail!("terminal approval timed out; no spaces installed; start a new link");
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
}

/// Create or resume a durable request, obtain one complete signed decision, and
/// confirm all selected spaces before publishing any aliases.
pub async fn execute(options: LinkOptions) -> Result<Vec<TerminalSpace>> {
    let store = SpaceStore::open()?;
    ensure!(
        !options.convert_account
            || (options.resume.is_none()
                && options.cancel.is_none()
                && options.expected_account.is_none()),
        "conversion requires a new request without a separate account constraint"
    );
    let now = dialog_ucan_core::time::Timestamp::now().to_unix();
    if let Some(id) = &options.cancel {
        let link = TerminalLink::resume(store, id, now).await?;
        link.cancel()?;
        println!(
            "Terminal request cancelled locally. No spaces were installed. Cancellation does not revoke grants already published by the browser."
        );
        return Ok(vec![]);
    }
    let timeout = options.timeout_seconds.unwrap_or(REQUEST_TTL_SECONDS);
    ensure!(
        (1..=REQUEST_TTL_SECONDS).contains(&timeout),
        "terminal timeout must be between 1 and 600 seconds"
    );
    let captured = if options.convert_account {
        let profile = crate::identity::open().await?;
        let operator =
            crate::account_state::credential_operator_for_store(&profile, &store).await?;
        let state = crate::account_session::snapshot(&profile, &operator, &store).await?;
        ensure!(
            state.pending_login.is_none(),
            "finish the pending account transition before converting"
        );
        Some(
            state
                .active
                .context("no active account attachment to convert")?,
        )
    } else {
        None
    };
    let (link, deployment) = if let Some(id) = &options.resume {
        let link = TerminalLink::resume(store, id, now).await?;
        let saved_origin = origin(&journal(&link.root)?.origin)?;
        if let Some(via) = &options.via {
            ensure!(
                origin(via)? == saved_origin,
                "terminal resume deployment differs from retained request"
            );
        }
        let deployment = discover(saved_origin).await?;
        ensure!(
            link.request.service() == &deployment.service,
            "terminal service identity changed since request creation"
        );
        (link, deployment)
    } else {
        let explicit = options
            .via
            .clone()
            .or_else(|| std::env::var(crate::deployment::CONNECTION_ORIGIN_ENV).ok());
        let deployment = discover(origin(
            explicit
                .as_deref()
                .unwrap_or(crate::account::DEFAULT_LINK_PAGE),
        )?)
        .await?;
        let expected = captured
            .as_ref()
            .map(|account| &account.root_did)
            .or(options.expected_account.as_ref())
            .map(String::as_str)
            .map(str::parse::<Did>)
            .transpose()
            .context("invalid explicitly requested account")?;
        let link = TerminalLink::create(
            store,
            &deployment.origin,
            &deployment.service,
            options.label.as_deref().unwrap_or("Terminal connection"),
            expected.as_ref(),
            now,
        )
        .await?;
        if let Some(captured) = captured {
            let _guard = lock(&link.root)?;
            let mut saved = journal(&link.root)?;
            saved.conversion = Some(captured);
            save(&link.root, &saved)?;
        }
        (link, deployment)
    };
    println!("Terminal request: {}", link.request.id());
    println!("Resume: tonk link --resume {}", link.request.id());
    if now < link.request.deadline()
        && matches!(link.state()?, LinkState::Pending | LinkState::Interrupted)
    {
        let url = link.url()?;
        println!("Approve selected spaces:");
        println!("{url}");
        std::io::stdout().flush()?;
        if !options.no_open {
            let _ = webbrowser::open(url.as_str());
        }
    }
    let previously_completed = link.state()? == LinkState::Completed;
    if previously_completed {
        finish_conversion(&link, true).await?;
    }
    let result = tokio::select! {
        result = wait(&link, &deployment, Duration::from_secs(timeout)) => result,
        _ = tokio::signal::ctrl_c() => {
            link.cancel()?;
            Err(anyhow::anyhow!("terminal request cancelled locally; no spaces installed; cancellation does not revoke published grants"))
        }
    };
    match result {
        Ok(spaces) => {
            finish_conversion(&link, false).await.context(format!(
                "scoped replicas retained; resume conversion with `tonk link --resume {}`",
                link.request.id()
            ))?;
            if previously_completed {
                println!(
                    "Terminal deliveries checked. {} local replica(s) retained; remote access is checked when used.",
                    spaces.len()
                );
            } else {
                println!(
                    "Terminal connection confirmed: {} selected space(s).",
                    spaces.len()
                );
            }
            println!(
                "Approving account: {}",
                link.approving_account()?.unwrap_or_default()
            );
            for space in &spaces {
                println!("  {} -> {}", space.name, space.site.display());
            }
            if journal(&link.root)?.conversion.is_some() {
                println!(
                    "The captured account attachment is inactive. Existing legacy aliases, paths, and unsynced edits remain local/offline; edits were not moved to these new scoped replicas."
                );
            }
            Ok(spaces)
        }
        Err(error) => {
            link.interrupt()?;
            if matches!(
                link.state()?,
                LinkState::Pending
                    | LinkState::Interrupted
                    | LinkState::Installing
                    | LinkState::Completed
            ) {
                Err(error.context(format!(
                    "terminal request retained; resume with `tonk link --resume {}`",
                    link.request.id()
                )))
            } else {
                Err(error)
            }
        }
    }
}

async fn finish_conversion(link: &TerminalLink, confirm_retained: bool) -> Result<()> {
    let _guard = lock(&link.root)?;
    let mut saved = journal(&link.root)?;
    let Some(captured) = saved.conversion.as_ref() else {
        return Ok(());
    };
    if saved.conversion_completed {
        return Ok(());
    }
    ensure!(
        saved.state == LinkState::Completed
            && saved.approving_account.as_deref() == Some(captured.root_did.as_str())
            && link.request.expected_account().map(|did| did.as_str())
                == Some(captured.root_did.as_str()),
        "conversion requires the complete verified selection from the captured account"
    );
    current_approval(
        &hex::decode(
            saved
                .approval
                .as_ref()
                .context("conversion approval missing")?,
        )?,
        dialog_ucan_core::time::Timestamp::now().to_unix(),
    )
    .await
    .context("conversion grants are no longer valid; the active attachment was retained")?;
    if confirm_retained {
        for space in &saved.spaces {
            let site =
                connections::open_published(&space.site, &space.connection, link.store.clone())
                    .await?;
            crate::handoff::confirm_scoped_connection(&site, &space.connection.id)
                .await
                .context(
                    "conversion access could not be confirmed; the active attachment was retained",
                )?;
        }
    }
    let profile = crate::identity::open().await?;
    let operator =
        crate::account_state::credential_operator_for_store(&profile, &link.store).await?;
    crate::account_session::deactivate_converted_attachment(
        &profile,
        &operator,
        &link.store,
        captured,
    )
    .await?;
    saved.conversion_completed = true;
    save(&link.root, &saved)
}
