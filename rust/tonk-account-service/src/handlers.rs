//! HTTP handlers: thin adapters from worker requests onto the core
//! ceremony logic.

pub mod health;
pub mod info;

#[cfg(target_arch = "wasm32")]
pub mod accounts;
#[cfg(target_arch = "wasm32")]
pub mod chains;
#[cfg(target_arch = "wasm32")]
pub mod codes;
#[cfg(target_arch = "wasm32")]
pub mod devices;
#[cfg(target_arch = "wasm32")]
pub mod links;

/// Add CORS headers permitting cross-origin requests to a response.
#[cfg(target_arch = "wasm32")]
pub fn with_cors_headers(response: worker::Response) -> worker::Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "POST, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Access-Control-Expose-Headers", "Content-Type");
    response.with_headers(headers)
}

/// Map a [`crate::core::CeremonyError`] onto a
/// [`crate::error::ServiceError`]. Ceremony-level messages (`Invalid`,
/// `Unauthorized`, `Forbidden`, `Conflict`) pass through: they are
/// written for callers. `Internal` detail is library/store error text
/// that must not reach the wire — it is logged here and replaced with a
/// generic message.
///
/// Shared by the wasm handlers and the native helpers server
/// ([`crate::helpers::server`]) so the two backends can't drift apart
/// on error mapping.
#[cfg(any(
    target_arch = "wasm32",
    all(feature = "helpers", not(target_arch = "wasm32"))
))]
pub fn ceremony_error(err: crate::core::CeremonyError) -> crate::error::ServiceError {
    use crate::core::CeremonyError;
    let message = match &err {
        CeremonyError::RateLimited => "rate limited".to_string(),
        CeremonyError::CodeInvalid => "invalid or expired code".to_string(),
        CeremonyError::Conflict(msg)
        | CeremonyError::Invalid(msg)
        | CeremonyError::Unauthorized(msg)
        | CeremonyError::Forbidden(msg) => msg.clone(),
        CeremonyError::Internal(detail) => {
            #[cfg(target_arch = "wasm32")]
            worker::console_error!("internal error: {detail}");
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!("internal error: {detail}");
            "internal error".to_string()
        }
    };
    crate::error::ServiceError::new(err.code(), message)
}

/// Read a request's body as raw bytes, mapping a failure onto a
/// [`crate::error::ServiceError`].
#[cfg(target_arch = "wasm32")]
pub async fn read_body(
    req: &mut worker::Request,
) -> std::result::Result<Vec<u8>, crate::error::ServiceError> {
    req.bytes().await.map_err(|err| {
        crate::error::ServiceError::new(
            crate::error::ErrorCode::InvalidArgument,
            format!("failed to read request body: {err}"),
        )
    })
}

/// Build a [`crate::store::d1::D1Store`] from the worker environment's
/// `DB` binding.
#[cfg(target_arch = "wasm32")]
pub fn build_store(
    ctx: &worker::RouteContext<()>,
) -> std::result::Result<crate::store::d1::D1Store, crate::error::ServiceError> {
    let db = ctx.env.d1("DB").map_err(|err| {
        crate::error::ServiceError::new(
            crate::error::ErrorCode::InternalError,
            format!("missing D1 binding: {err}"),
        )
    })?;
    Ok(crate::store::d1::D1Store::new(db))
}

/// Build a [`crate::chains::r2::R2ChainStore`] from the worker
/// environment's `CHAINS` binding.
#[cfg(target_arch = "wasm32")]
pub fn build_chains(
    ctx: &worker::RouteContext<()>,
) -> std::result::Result<crate::chains::r2::R2ChainStore, crate::error::ServiceError> {
    let bucket = ctx.bucket("CHAINS").map_err(|err| {
        crate::error::ServiceError::new(
            crate::error::ErrorCode::InternalError,
            format!("missing R2 binding: {err}"),
        )
    })?;
    Ok(crate::chains::r2::R2ChainStore::new(bucket))
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::ceremony_error;
    use crate::core::CeremonyError;
    use crate::error::ErrorCode;

    #[dialog_common::test]
    async fn it_hides_internal_detail_from_the_wire() {
        let err = ceremony_error(CeremonyError::Internal(
            "R2 bucket 'tonk-account-chains' unreachable".into(),
        ));
        assert_eq!(err.code, ErrorCode::InternalError);
        assert_eq!(err.message, "internal error");
    }

    #[dialog_common::test]
    async fn it_passes_ceremony_messages_through() {
        let err = ceremony_error(CeremonyError::Forbidden(
            "device is not an active member of this account".into(),
        ));
        assert_eq!(err.code, ErrorCode::Forbidden);
        assert_eq!(
            err.message,
            "device is not an active member of this account"
        );
    }
}
