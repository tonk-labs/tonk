//! Spaces an earlier release kept in the app's origin, which no origin of the
//! kernel build can read. The app's page reads each one kept on that device
//! alone and hands it to the persona's host, which keeps it; the persona
//! makes a space as a copy of it, and the new space's own worker is created
//! from what the host keeps, as a copy of another space is.

use tonk_common::log;

use crate::TonkWorkerError;
use crate::router::AppState;

/// Make a space named `name` as a copy of `copy`, what the persona's host
/// keeps of a space an earlier release kept. Answers the new space's DID.
pub async fn bring_over(
    state: &AppState,
    name: &str,
    copy: &str,
) -> Result<String, TonkWorkerError> {
    log!("bringing over '{name}' as a copy of {copy}");
    let made = super::repository::create_migrated_copy(state, name, copy)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()));
    log!("bringing over '{name}': {made:?}");
    made
}
