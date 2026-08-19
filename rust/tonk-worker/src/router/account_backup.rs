//! The account provider's base URL — the one thing the retired escrow
//! module still answers for. The spot-backup escrow itself is gone:
//! the account DB's directory facts are the source of truth for which
//! spaces exist and how to mount them (see `adopt`), and the account
//! service shrinks toward custody and revocations.

use crate::worker::TonkState;

/// The linked account provider's base URL, when an account is attached.
pub(crate) async fn account_service_url(tonk: &TonkState) -> Option<String> {
    crate::router::account::provider(tonk).await
}
