//! The email verification code ceremony: request a code, then verify it.

use crate::core::CeremonyError;
use crate::email::EmailSender;
use crate::store::{CodeRow, Store};

/// How long a requested code remains valid, in seconds.
pub const CODE_TTL_SECONDS: u64 = 600;

/// Minimum time between two code requests for the same email, in
/// seconds.
pub const RESEND_COOLDOWN_SECONDS: u64 = 60;

/// Maximum number of verification attempts allowed against a single
/// code before it is treated as exhausted.
pub const MAX_ATTEMPTS: u32 = 5;

/// Generate a fresh six-digit, zero-padded verification code.
pub fn generate_code() -> String {
    format!("{:06}", rand::random::<u32>() % 1_000_000)
}

/// Hash a `(email, code)` pair for storage. Never store the code
/// itself.
pub fn hash_code(email: &str, code: &str) -> String {
    blake3::hash(format!("{email}:{code}").as_bytes())
        .to_hex()
        .to_string()
}

/// Request a verification code for `email`, sending it via `sender`.
///
/// Refuses a second send inside [`RESEND_COOLDOWN_SECONDS`] of the
/// stored row's `created_at`. The email address is lowercased before
/// every store access.
pub async fn request_code<S: Store, E: EmailSender>(
    store: &S,
    sender: &E,
    email: &str,
    code: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    let email = email.to_lowercase();
    if let Some(existing) = store.code(&email).await? {
        let elapsed = now.saturating_sub(existing.created_at);
        if elapsed < RESEND_COOLDOWN_SECONDS {
            return Err(CeremonyError::RateLimited);
        }
    }
    store
        .put_code(&CodeRow {
            email: email.clone(),
            code_hash: hash_code(&email, code),
            created_at: now,
            expires_at: now + CODE_TTL_SECONDS,
            attempts: 0,
        })
        .await?;
    sender
        .send_code(&email, code)
        .await
        .map_err(|err| CeremonyError::Internal(format!("{err:?}")))?;
    Ok(())
}

/// Check a previously requested code without consuming it.
///
/// Returns [`CeremonyError::CodeInvalid`] uniformly when there is no
/// pending code, the code has expired, attempts are exhausted, or the
/// supplied code does not match — so responses don't reveal which
/// check failed. The email address is lowercased before every store
/// access.
pub async fn check_code<S: Store>(
    store: &S,
    email: &str,
    code: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    let email = email.to_lowercase();
    let Some(row) = store.code(&email).await? else {
        return Err(CeremonyError::CodeInvalid);
    };
    if now >= row.expires_at || row.attempts >= MAX_ATTEMPTS {
        return Err(CeremonyError::CodeInvalid);
    }
    if row.code_hash != hash_code(&email, code) {
        store.bump_attempts(&email).await?;
        return Err(CeremonyError::CodeInvalid);
    }
    Ok(())
}

/// Verify a previously requested code, consuming it on success.
pub async fn verify_code<S: Store>(
    store: &S,
    email: &str,
    code: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    check_code(store, email, code, now).await?;
    let email = email.to_lowercase();
    store.delete_code(&email).await?;
    Ok(())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::email::CapturedEmail;
    use crate::store::sqlite::SqliteStore;

    #[dialog_common::test]
    async fn it_delivers_a_code_and_verifies_it_once() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "A@X.com", "123456", 100)
            .await
            .unwrap();
        let sent = sender.0.lock().unwrap().clone();
        assert_eq!(sent, vec![("a@x.com".to_string(), "123456".to_string())]);
        verify_code(&store, "a@x.com", "123456", 200).await.unwrap();
        // consumed: the same code no longer verifies
        assert!(matches!(
            verify_code(&store, "a@x.com", "123456", 201).await,
            Err(CeremonyError::CodeInvalid)
        ));
    }

    #[dialog_common::test]
    async fn it_rate_limits_resends_inside_the_cooldown() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "111111", 100)
            .await
            .unwrap();
        assert!(matches!(
            request_code(&store, &sender, "a@x.com", "222222", 130).await,
            Err(CeremonyError::RateLimited)
        ));
        request_code(
            &store,
            &sender,
            "a@x.com",
            "222222",
            100 + RESEND_COOLDOWN_SECONDS,
        )
        .await
        .unwrap();
    }

    #[dialog_common::test]
    async fn it_rejects_expired_wrong_and_exhausted_codes() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        assert!(matches!(
            verify_code(&store, "a@x.com", "123456", 100 + CODE_TTL_SECONDS).await,
            Err(CeremonyError::CodeInvalid)
        ));
        request_code(&store, &sender, "b@x.com", "123456", 100)
            .await
            .unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert!(matches!(
                verify_code(&store, "b@x.com", "000000", 200).await,
                Err(CeremonyError::CodeInvalid)
            ));
        }
        // right code, but attempts are spent
        assert!(matches!(
            verify_code(&store, "b@x.com", "123456", 200).await,
            Err(CeremonyError::CodeInvalid)
        ));
    }
}
