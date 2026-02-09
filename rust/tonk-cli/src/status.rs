use anyhow::{Context, Result};

/// Show the current context: operator, session, space, remote state.
pub async fn execute(json: bool) -> Result<()> {
    let keystore = crate::keystore::Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.did().to_string();

    // Get active authority
    let authority = crate::authority::get_active_authority()?;
    let authority_did = authority.as_ref().map(|a| a.did.clone());

    // Get active space
    let space_info = if let Some(auth) = &authority {
        if let Some(space_did) = crate::state::get_active_space(&auth.did)? {
            let name = crate::metadata::SpaceMetadata::load(&space_did)
                .ok()
                .flatten()
                .map(|m| m.name);
            Some((space_did, name))
        } else {
            None
        }
    } else {
        None
    };

    if json {
        let output = serde_json::json!({
            "operator": operator_did,
            "session": authority_did,
            "space": space_info.as_ref().map(|(did, name)| {
                serde_json::json!({
                    "did": did,
                    "name": name,
                })
            }),
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("🫆 Operator:  {}", operator_did);

        if let Some(auth_did) = &authority_did {
            println!("👤 Authority: {}", auth_did);
        } else {
            println!("👤 Authority: (none - run 'tonk login')");
        }

        if let Some((space_did, name)) = &space_info {
            if let Some(name) = name {
                println!("🏠 Space:     {} ({})", name, space_did);
            } else {
                println!("🏠 Space:     {}", space_did);
            }
        } else {
            println!("🏠 Space:     (none - run 'tonk space create')");
        }
        println!();
    }

    Ok(())
}
