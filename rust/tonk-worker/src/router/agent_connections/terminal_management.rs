//! Public terminal management and authenticated delivery to its existing key.
use super::*;
use tonk_invite::terminal::{Addition, Approval};
use tonk_worker_api::{
    TerminalConnectionAddReceipt, TerminalConnectionAddRequest, TerminalConnectionRevokeRequest,
    TerminalConnectionSummary,
};

fn approval_entity(request: &str) -> Result<dialog_artifacts::Entity, TonkWorkerError> {
    if request.len() != 64 || !request.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failure("invalid terminal request ID"));
    }
    format!("id:tonk:terminal-approval:{request}")
        .parse()
        .map_err(failure)
}

async fn initial(tonk: &TonkState, request: &str) -> Result<Approval, TonkWorkerError> {
    let root = super::super::identity::local_root(tonk).await?;
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<fields::TerminalLinkApproval> = branch
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkApproval> {
            this: Term::from(approval_entity(request)?),
            account: Term::from(fields::Account(root.root_did.to_string())),
            approval: Term::var("approval"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    if rows.len() != 1 {
        return Err(TonkWorkerError::NotFound(
            "terminal connection is not in this account".into(),
        ));
    }
    let approval = Approval::inspect(&hex::decode(&rows[0].approval.0).map_err(failure)?)
        .await
        .map_err(failure)?;
    if approval.request().id() != request
        || approval.account() != &root.root_did
        || approval.is_declined()
    {
        return Err(failure(
            "saved terminal approval does not match this account",
        ));
    }
    Ok(approval)
}

async fn delivered(
    tonk: &TonkState,
    entity: dialog_artifacts::Entity,
    field: &str,
    request: &str,
) -> Result<bool, TonkWorkerError> {
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<fields::TerminalLinkDelivered> = branch
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkDelivered> {
            this: Term::from(entity),
            delivery_receipt: Term::var("receipt"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    Ok(rows.iter().any(|row| {
        serde_json::from_str::<serde_json::Value>(&row.delivery_receipt.0).is_ok_and(|receipt| {
            receipt.get(field).and_then(|value| value.as_str()) == Some(request)
                && receipt
                    .get("recorded")
                    .and_then(|value| value.as_bool())
                    .is_some()
        })
    }))
}

async fn pending_additions(
    tonk: &TonkState,
    initial: &Approval,
) -> Result<Vec<tonk_worker_api::TerminalPendingAddition>, TonkWorkerError> {
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<fields::TerminalLinkAddition> = branch
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkAddition> {
            this: Term::var("this"),
            account: Term::from(fields::Account(initial.account().to_string())),
            addition: Term::var("addition"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let mut pending = Vec::new();
    for row in rows {
        let addition = Addition::inspect(&hex::decode(&row.addition.0).map_err(failure)?)
            .await
            .map_err(failure)?;
        if addition.account() != initial.account() {
            return Err(failure("saved addition account mismatch"));
        }
        if addition.request_id() != initial.request().id() {
            continue;
        }
        if addition.recipient() != initial.request().recipient()
            || addition.service() != initial.request().service()
        {
            return Err(failure("saved addition terminal identity mismatch"));
        }
        if !delivered(tonk, row.this.clone(), "deliveryId", &addition.id()).await? {
            let key = row.this.to_string();
            let prefix = format!("id:tonk:terminal-addition:{}:", initial.request().id());
            let operation_id = key
                .strip_prefix(&prefix)
                .ok_or_else(|| failure("saved addition operation identity mismatch"))?;
            pending.push(tonk_worker_api::TerminalPendingAddition {
                operation_id: operation_id.into(),
                delivery_id: addition.id(),
                subjects: addition
                    .bundles()
                    .iter()
                    .map(|bundle| bundle.subject().to_string())
                    .collect(),
            });
        }
    }
    Ok(pending)
}

#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<TerminalConnectionSummary>>, TonkWorkerError> {
    enabled()?;
    let tonk = state.read().await;
    let root = super::super::identity::local_root(&tonk).await?;
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<fields::TerminalLinkApproval> = branch
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkApproval> {
            this: Term::var("this"),
            account: Term::from(fields::Account(root.root_did.to_string())),
            approval: Term::var("approval"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let all = groups(&tonk).await?;
    let mut result = Vec::new();
    for row in rows {
        let approval = Approval::inspect(&hex::decode(&row.approval.0).map_err(failure)?)
            .await
            .map_err(failure)?;
        if approval.account() != &root.root_did
            || row.this != approval_entity(&approval.request().id())?
        {
            return Err(failure("terminal account or request mismatch"));
        }
        if approval.is_declined() {
            continue;
        }
        let mut spaces = Vec::new();
        for group in all.iter().filter(|group| {
            group.terminal_request.as_deref() == Some(approval.request().id().as_str())
        }) {
            if group.recipient != approval.request().recipient().as_str() {
                return Err(failure("terminal recipient mismatch"));
            }
            spaces.push(summarize(&tonk, group).await?);
        }
        spaces.sort_by(|left, right| {
            left.subject
                .cmp(&right.subject)
                .then(left.id.cmp(&right.id))
        });
        let pending_additions = pending_additions(&tonk, &approval).await?;
        let all_delivered = pending_additions.is_empty();
        result.push(TerminalConnectionSummary {
            pending_additions,
            request_id: approval.request().id(),
            recipient: approval.request().recipient().to_string(),
            label: approval.request().label().into(),
            account: approval.account().to_string(),
            delivery_status: if delivered(&tonk, row.this, "requestId", &approval.request().id())
                .await?
                && all_delivered
            {
                "delivered"
            } else {
                "pending"
            }
            .into(),
            spaces,
        });
    }
    result.sort_by(|left, right| left.request_id.cmp(&right.request_id));
    Ok(Json(result))
}

/// Republish only a durably saved complete initial approval. Never mint grants.
#[wasm_compat]
pub async fn retry(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Result<Json<tonk_worker_api::TerminalLinkApprovalReceipt>, TonkWorkerError> {
    enabled()?;
    let _serial = TERMINAL_APPROVAL.lock().await;
    let tonk = state.write().await;
    let saved = initial(&tonk, &request_id).await?;
    let approval = Approval::validate(saved.bytes(), Timestamp::now().to_unix())
        .await
        .map_err(failure)?;
    let endpoint = terminal_endpoint(&tonk, approval.request()).await?;
    publish_terminal_approval(&tonk, &approval, approval_entity(&request_id)?, &endpoint)
        .await
        .map(Json)
}

#[wasm_compat]
pub async fn add(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(input): Json<TerminalConnectionAddRequest>,
) -> Result<Json<TerminalConnectionAddReceipt>, TonkWorkerError> {
    enabled()?;
    let _serial = TERMINAL_APPROVAL.lock().await;
    let tonk = state.write().await;
    if input.operation_id.is_empty()
        || input.operation_id.len() > 64
        || !input
            .operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || input.subjects.is_empty()
        || input.subjects.len() > tonk_invite::terminal::MAX_SELECTED_SPACES
    {
        return Err(failure(
            "invalid terminal addition selection or operation ID",
        ));
    }
    let approval = initial(&tonk, &request_id).await?;
    let root = super::super::identity::local_root(&tonk).await?;
    let endpoint = terminal_endpoint(&tonk, approval.request())
        .await?
        .join("/connection/addition")
        .map_err(failure)?;
    let mut subjects = input.subjects;
    subjects.sort();
    if subjects.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(failure("duplicate selected space"));
    }
    let entity: dialog_artifacts::Entity = format!(
        "id:tonk:terminal-addition:{request_id}:{}",
        input.operation_id
    )
    .parse()
    .map_err(failure)?;
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<fields::TerminalLinkAddition> = branch
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkAddition> {
            this: Term::from(entity.clone()),
            account: Term::from(fields::Account(root.root_did.to_string())),
            addition: Term::var("addition"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    if rows.len() > 1 {
        return Err(failure("conflicting terminal addition retry"));
    }
    let now = Timestamp::now();
    let addition = if let Some(row) = rows.first() {
        let addition = Addition::validate(
            &hex::decode(&row.addition.0).map_err(failure)?,
            now.to_unix(),
        )
        .await
        .map_err(failure)?;
        let saved: Vec<_> = addition
            .bundles()
            .iter()
            .map(|bundle| bundle.subject().to_string())
            .collect();
        if addition.request_id() != request_id
            || addition.account() != &root.root_did
            || addition.recipient() != approval.request().recipient()
            || saved != subjects
        {
            return Err(TonkWorkerError::Conflict(
                "a different complete addition was already staged for this action".into(),
            ));
        }
        addition
    } else {
        let snapshot = selection_snapshot(&tonk).await?;
        if snapshot.snapshot != input.snapshot || snapshot.account != root.root_did.as_str() {
            return Err(TonkWorkerError::Conflict(
                "account or spaces changed; review the addition again".into(),
            ));
        }
        for group in groups(&tonk).await?.into_iter().filter(|group| {
            group.terminal_request.as_deref() == Some(request_id.as_str())
                && subjects.contains(&group.subject)
        }) {
            let current = summarize(&tonk, &group).await?;
            if current.status != "revoked" && current.status != "expired" {
                return Err(TonkWorkerError::Conflict("this terminal already has active or partially revoked access to a selected space".into()));
            }
        }
        let (bundles, selected_groups) =
            prepare_terminal_selection(&tonk, &snapshot, &subjects, approval.request(), now)
                .await?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(failure)?;
        let addition = Addition::sign(
            tonk.profile.signer().signer(),
            &approval,
            root.delegation,
            bundles,
            nonce,
            now.to_unix(),
        )
        .await
        .map_err(failure)?;
        branch
            .handle()
            .delegations()
            .retain_all(
                addition
                    .bundles()
                    .iter()
                    .flat_map(|bundle| bundle.chains().iter().cloned().map(UcanDelegation))
                    .collect::<Vec<_>>(),
            )
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        let mut transaction = tonk
            .reactor
            .profile_repository()
            .branch("main")
            .transaction();
        for group in selected_groups {
            transaction = transaction.assert(AgentGrantGroup {
                this: group_entity(&group.id)?,
                account: fields::Account(group.account.clone()),
                subject: fields::Subject(group.subject.clone()),
                public_record: fields::PublicRecord(
                    serde_json::to_string(&group).map_err(failure)?,
                ),
            });
        }
        transaction
            .assert(fields::TerminalLinkAddition {
                this: entity.clone(),
                account: fields::Account(root.root_did.to_string()),
                addition: fields::Addition(hex::encode(addition.bytes())),
            })
            .commit()
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        addition
    };
    let response = super::super::http::terminal_request(&endpoint, Some(addition.bytes())).await?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Receipt {
        delivery_id: String,
        recorded: bool,
    }
    let receipt: Receipt = serde_json::from_slice(&response.body).map_err(failure)?;
    if receipt.delivery_id != addition.id() {
        return Err(failure("service acknowledged another terminal addition"));
    }
    record_terminal_delivery(&tonk, entity, &response.body).await?;
    let ids: Vec<_> = addition
        .bundles()
        .iter()
        .map(|bundle| {
            grant_set_id(
                bundle.subject().as_str(),
                addition.recipient().as_str(),
                &cids(bundle),
            )
        })
        .collect();
    let mut connections = Vec::new();
    for group in groups(&tonk)
        .await?
        .iter()
        .filter(|group| ids.contains(&group.id))
    {
        connections.push(summarize(&tonk, group).await?);
    }
    if connections.len() != subjects.len() {
        return Err(failure("saved terminal addition groups are incomplete"));
    }
    Ok(Json(TerminalConnectionAddReceipt {
        delivery_id: receipt.delivery_id,
        recorded: receipt.recorded,
        connections,
    }))
}

#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(input): Json<TerminalConnectionRevokeRequest>,
) -> Result<Json<Vec<AgentConnectionSummary>>, TonkWorkerError> {
    enabled()?;
    let _serial = TERMINAL_APPROVAL.lock().await;
    let tonk = state.write().await;
    let approval = initial(&tonk, &request_id).await?;
    let all: Vec<_> = groups(&tonk)
        .await?
        .into_iter()
        .filter(|group| {
            group.terminal_request.as_deref() == Some(request_id.as_str())
                && group.recipient == approval.request().recipient().as_str()
        })
        .collect();
    if let Some(ids) = &input.group_ids
        && (ids.is_empty()
            || ids.len() > 1024
            || ids
                .iter()
                .any(|id| !all.iter().any(|group| &group.id == id)))
    {
        return Err(failure(
            "revocation selection contains a group outside this terminal",
        ));
    }
    let mut result = Vec::new();
    for group in all.iter().filter(|group| {
        input
            .group_ids
            .as_ref()
            .is_none_or(|ids| ids.contains(&group.id))
    }) {
        result.push(
            revoke_group(&tonk, group, |chain, cid| {
                let tonk = &*tonk;
                async move { publish_target(tonk, group, &chain, &cid).await }
            })
            .await?,
        );
    }
    Ok(Json(result))
}
