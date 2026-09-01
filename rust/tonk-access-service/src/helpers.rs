//! Test helpers for the UCAN access service.
//!
//! This module provides a local UCAN access service for integration testing.
//! It mirrors the behavior of the Cloudflare Worker but runs as a native HTTP
//! server, allowing tests to run without deploying to Cloudflare.

use serde::{Deserialize, Serialize};

/// Connection info for the UCAN access service test server.
///
/// Contains all information needed to configure `ucan::Credentials` and
/// connect to the backing S3 server for test verification.
///
/// This struct is available on all platforms so it can be used as a test
/// parameter in WASM tests, even though the server only runs natively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessServiceAddress {
    /// URL of the UCAN access service (e.g., "http://127.0.0.1:8080")
    pub access_service_url: String,
    /// URL of the backing S3 server (for test verification)
    pub s3_endpoint: String,
    /// The bucket name
    pub bucket: String,
    /// AWS access key ID (used by access service, exposed for verification)
    pub access_key_id: String,
    /// AWS secret access key (used by access service, exposed for verification)
    pub secret_access_key: String,
    /// The service's signing DID, issuer of activation delegations.
    pub service_did: String,
}

/// Enrolling and activating a customer against a test service.
///
/// The provisioning gate serves a subject only while an active customer
/// pays for it, so a test that presigns anything needs its subject's
/// customer past email confirmation first. Doing that by hand is three
/// round trips of ceremony that has nothing to do with what most tests
/// are checking.
#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
impl AccessServiceAddress {
    /// The `/ucan/` endpoint of this service.
    pub fn ucan_endpoint(&self) -> String {
        format!("{}/ucan/", self.access_service_url.trim_end_matches('/'))
    }

    /// Make `subject` servable without running the registration
    /// ceremony: enroll a customer, activate it, and provision `subject`
    /// under it, all straight against the control store.
    ///
    /// For tests whose subject is a repository or space DID they hold no
    /// signer for, and whose point is not registration. A test that is
    /// about the ceremony itself should drive the real endpoints
    /// instead — see [`Self::activate_customer`].
    pub async fn provision_subject(&self, subject: &str) -> anyhow::Result<()> {
        let response = reqwest::Client::new()
            .post(format!(
                "{}/_test/provision",
                self.access_service_url.trim_end_matches('/')
            ))
            .json(&serde_json::json!({ "subject": subject }))
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "provisioning {subject} failed ({}): {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
        Ok(())
    }

    /// Enroll `customer` under `email`, leaving it `Registered`: the
    /// address is claimed, and the activation email is waiting in the
    /// test inbox. [`Self::activate_customer`] is this plus confirming
    /// it; a test that wants a customer stopped short of confirmation
    /// wants this one.
    pub async fn enroll_customer(
        &self,
        customer: &dialog_credentials::Ed25519Signer,
        email: &str,
    ) -> anyhow::Result<()> {
        self.enroll_only(customer, email).await
    }

    /// Enroll `customer` and confirm its email, leaving it `Active` and
    /// providing its own account space. Returns once the service has
    /// recorded the activation, so a presign on `customer`'s subject
    /// immediately afterwards is served.
    pub async fn activate_customer(
        &self,
        customer: &dialog_credentials::Ed25519Signer,
        email: &str,
    ) -> anyhow::Result<()> {
        self.enroll_only(customer, email).await?;
        self.confirm_email(email).await
    }

    /// The enrollment half of the ceremony: mint a device delegation and
    /// the service deposits, and invoke `/customer/enroll`.
    async fn enroll_only(
        &self,
        customer: &dialog_credentials::Ed25519Signer,
        email: &str,
    ) -> anyhow::Result<()> {
        use dialog_varsig::Principal as _;

        let device = dialog_credentials::Ed25519Signer::generate()
            .await
            .map_err(|error| anyhow::anyhow!("device signer: {error:?}"))?;
        let link =
            tonk_identity::delegation::mint_device_delegation(customer.clone(), &device.did())
                .await?;
        // No ceremony here, so the harness mints its own custody set:
        // every enrollment must present one, and these tests are about
        // the customer lifecycle rather than custody itself.
        let custody_key = dialog_credentials::Ed25519Signer::generate()
            .await
            .map_err(|error| anyhow::anyhow!("custody signer: {error:?}"))?;
        let custody = tonk_identity::request::mint_custody_material(
            &custody_key,
            &customer.did(),
            b"sealed-account-secret".to_vec(),
        )
        .await?;
        let container = tonk_identity::request::build_enroll_invocation(
            device,
            &link,
            email,
            &custody.borrow(),
        )
        .await?;

        let client = reqwest::Client::new();
        let endpoint = self.ucan_endpoint();
        let response = client
            .post(&endpoint)
            .header("Content-Type", "application/cbor")
            .body(container)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "enrollment refused ({}): {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
        Ok(())
    }

    /// The confirmation half: present the invocation the activation email
    /// carries. It is complete and service-signed, so presenting it is
    /// activating and no key is needed here. Public so a lifecycle test
    /// can confirm at a chosen moment — "another device opened the
    /// link" — rather than only as part of [`Self::activate_customer`].
    pub async fn confirm_email(&self, email: &str) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let endpoint = self.ucan_endpoint();
        let inbox: Vec<(String, String)> = client
            .get(format!(
                "{}/_test/emails",
                self.access_service_url.trim_end_matches('/')
            ))
            .send()
            .await?
            .json()
            .await?;
        // Enrollment normalizes the address before storing and sending,
        // so the inbox is keyed by the normalized form: matching the
        // caller's spelling verbatim would miss a mixed-case address.
        let normalized = crate::email::normalize_email(email);
        let (_, link) = inbox
            .into_iter()
            .rev()
            .find(|(to, _)| crate::email::normalize_email(to) == normalized)
            .ok_or_else(|| anyhow::anyhow!("no activation email was captured for {email}"))?;
        let encoded = link
            .split_once("ucan=")
            .map(|(_, value)| value)
            .ok_or_else(|| anyhow::anyhow!("the activation link carries no invocation"))?;
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, encoded)?;
        let response = client
            .post(&endpoint)
            .header("Content-Type", "application/cbor")
            .body(bytes)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "activation refused ({}): {}",
            response.status(),
            response.text().await.unwrap_or_default()
        );
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod server;

#[cfg(not(target_arch = "wasm32"))]
pub use server::*;

// Re-export SignerCredential for convenience in tests
#[cfg(not(target_arch = "wasm32"))]
pub use dialog_credentials::SignerCredential;
