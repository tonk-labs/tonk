//! A space's kernel acting for an agent, as the hosted agent's Worker boots
//! it: over layers of its own, as a writer key the host holds, handed the
//! agent's chain down to that key. It syncs with an access service, so what
//! one writer pushes, another, booted fresh, pulls.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;

use dialog_common::helpers::{Provisionable as _, Service};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use ipld_core::ipld::Ipld;
use tonk_access_service::helpers::{AccessServer, AccessServiceAddress};
use tonk_kernel_host::credentials::Standing;
use tonk_kernel_host::kernel::Kernel;
use tonk_kernel_host::layers::Layers;
use wcmp::Engine;

fn engine() -> Engine {
    Engine::with_backend(wcmp_wasm_core_wasmtime::Wasmtime::new().unwrap()).unwrap()
}

/// Layers no other kernel uses.
fn scratch() -> Layers {
    let path = std::env::temp_dir().join(format!("tonk-hosted-writer-{}", rand::random::<u64>()));
    Layers::under(path.to_string_lossy().into_owned())
}

/// An access service serving a fresh agent space.
struct Fixture {
    service: Service<AccessServiceAddress, AccessServer>,
    agent: Ed25519Signer,
    remote: url::Url,
}

impl Fixture {
    async fn start() -> Self {
        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let agent = Ed25519Signer::generate().await.unwrap();
        service
            .address
            .provision_subject(agent.did().as_ref())
            .await
            .unwrap();
        let remote = url::Url::parse(&service.address.ucan_endpoint()).unwrap();
        Self {
            service,
            agent,
            remote,
        }
    }

    /// A writer's seed, and the agent space's chain down to its key: the
    /// session's powerline, as an agent session token carries it, and the
    /// session's delegation to the writer, which says whom it acts for and
    /// where the space syncs.
    async fn writer(&self) -> ([u8; 32], Vec<u8>) {
        let session = Ed25519Signer::import(&rand::random::<[u8; 32]>())
            .await
            .unwrap();
        let powerline = tonk_identity::delegation::mint_addressed_device_delegation(
            self.agent.clone(),
            &session.did(),
            &self.remote,
        )
        .await
        .unwrap();
        let seed = rand::random::<[u8; 32]>();
        let writer = Ed25519Signer::import(&seed).await.unwrap();
        let mut meta: BTreeMap<String, Ipld> = tonk_invite::home_address_meta(&self.remote);
        meta.insert(
            "acts.for".to_owned(),
            Ipld::String(self.agent.did().to_string()),
        );
        let hop = DelegationBuilder::new()
            .issuer(Signer::from(session))
            .audience(&writer.did())
            .subject(Subject::Any)
            .command(vec![])
            .meta(meta)
            .try_build()
            .await
            .unwrap();
        let chain: DelegationChain = powerline.push(hop).unwrap();
        (seed, chain.to_bytes().unwrap())
    }

    /// A kernel of the agent space, booted fresh as a new writer.
    async fn kernel(&self, engine: &Engine) -> Kernel {
        let (seed, chain) = self.writer().await;
        let mut kernel = Kernel::load(engine, tonk_components::TONK_KERNEL_BYTES, scratch())
            .await
            .unwrap();
        kernel
            .init(
                self.agent.did().as_ref(),
                Standing::handed(seed, Some(chain)),
            )
            .await
            .unwrap();
        kernel
    }
}

const SCHEMA_AND_NOTE: &str = "attribute!: &hosted-note-title
  description: Title
  the: hosted.note/title
  as: text
concept!: &hosted-note
  description: A note
  with:
    title: hosted-note-title
hosted-note!:
  this: urn:hosted:note
  title: \"written by one writer\"
";

const NOTES: &str = "hosted.note:\n  this: ?note\n  title: ?title\n";

#[tokio::test(flavor = "current_thread")]
async fn it_pulls_what_another_writer_of_the_agent_space_pushed() {
    let engine = engine();
    let fixture = Fixture::start().await;
    let space = fixture.agent.did().to_string();
    let branch = format!("/api/repository/{space}/branch/main");

    let mut writer = fixture.kernel(&engine).await;
    let written = writer
        .request(
            "POST",
            &format!("{branch}/evaluate?transact=true"),
            SCHEMA_AND_NOTE.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(written.status, 200, "the write: {}", written.text());
    let pushed = writer
        .request("POST", &format!("{branch}/sync/push"), b"")
        .await
        .unwrap();
    assert_eq!(pushed.status, 200, "the push: {}", pushed.text());

    let mut reader = fixture.kernel(&engine).await;
    let pulled = reader
        .request("POST", &format!("{branch}/sync/pull"), b"")
        .await
        .unwrap();
    assert_eq!(pulled.status, 200, "the pull: {}", pulled.text());
    let read = reader
        .request(
            "POST",
            &format!("{branch}/evaluate?transact=false"),
            NOTES.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(read.status, 200, "the read: {}", read.text());
    assert!(
        read.text().contains("written by one writer"),
        "the reader sees the note: {}",
        read.text()
    );

    fixture.service.stop().await.unwrap();
}
