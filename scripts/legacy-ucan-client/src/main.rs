//! The actual v0.6.15 provider, isolated from current dialog types.
use dialog_capability::{ForkInvocation, Provider, Subject};
use dialog_common::Blake3Hash;
use dialog_effects::{
    Use,
    archive::{Archive, Catalog, Get},
    blob::{self, prelude::ArchiveBlobExt},
};
use dialog_remote_ucan_s3::{UcanAddress, UcanAuthorization, UcanInvocation, UcanSite};
use dialog_ucan_core::{Container, InvocationChain};
fn authorization(value: &serde_json::Value) -> UcanAuthorization {
    let bytes: Vec<u8> = serde_json::from_value(value.clone()).unwrap();
    let chain = InvocationChain::try_from(Container::from_bytes(&bytes).unwrap()).unwrap();
    let subject = chain.subject().clone();
    let ability = format!("/{}", chain.command().0.join("/"));
    UcanInvocation {
        chain: Box::new(chain),
        subject,
        ability,
    }
    .into()
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let address = UcanAddress::new(&args[2]);
    let site = UcanSite::default();
    let subject: dialog_capability::Did = fixture["subject"].as_str().unwrap().parse().unwrap();
    let bytes: Vec<u8> = serde_json::from_value(fixture["content"].clone())?;
    let digest = Blake3Hash::hash(&bytes);
    for (index, range) in [Some((0, 3)), Some((3, 3)), None, Some((1, 4))]
        .into_iter()
        .enumerate()
    {
        let effect = match range {
            Some((offset, length)) => blob::Read::range(digest.clone(), offset, Some(length)),
            None => blob::Read::new(digest.clone()),
        };
        let cap = Subject::from(subject.clone())
            .attenuate(Use)
            .attenuate(Archive)
            .blob()
            .invoke(effect);
        let mut result: blob::BlobReader = site
            .execute(ForkInvocation::new(
                cap,
                address.clone(),
                authorization(&fixture["ranges"][index]),
            ))
            .await?;
        let mut got = Vec::new();
        while let Some(chunk) = result.next().await? {
            got.extend(chunk);
        }
        let expected = match range {
            Some((offset, length)) => &bytes[offset as usize..(offset + length) as usize],
            None => &bytes,
        };
        anyhow::ensure!(
            got == expected,
            "old client's sequential range returned wrong bytes"
        );
    }
    let first = Subject::from(subject.clone())
        .attenuate(Use)
        .attenuate(Archive)
        .attenuate(Catalog::new("index"))
        .invoke(Get::new(digest));
    let second_bytes: Vec<u8> = serde_json::from_value(fixture["secondContent"].clone())?;
    let second = Subject::from(subject)
        .attenuate(Use)
        .attenuate(Archive)
        .attenuate(Catalog::new("index"))
        .invoke(Get::new(Blake3Hash::hash(&second_bytes)));
    let (a, b): (
        Result<Option<Vec<u8>>, dialog_effects::archive::ArchiveError>,
        Result<Option<Vec<u8>>, dialog_effects::archive::ArchiveError>,
    ) = tokio::join!(
        site.execute(ForkInvocation::new(
            first,
            address.clone(),
            authorization(&fixture["invocations"][0])
        )),
        site.execute(ForkInvocation::new(
            second,
            address.clone(),
            authorization(&fixture["secondBlock"])
        ))
    );
    anyhow::ensure!(
        a? == Some(bytes.clone()) && b? == Some(second_bytes),
        "concurrent old archive reads crossed object/authority identity"
    );
    // Cold different ranges share this old client's redeem cache key. A safe
    // server must reject the mismatched original invocation rather than alter it.
    let cold = UcanSite::default();
    let subject: dialog_capability::Did = fixture["subject"].as_str().unwrap().parse().unwrap();
    let cap = |offset| {
        Subject::from(subject.clone())
            .attenuate(Use)
            .attenuate(Archive)
            .blob()
            .invoke(blob::Read::range(Blake3Hash::hash(&bytes), offset, Some(3)))
    };
    let (a, b): (
        Result<blob::BlobReader, blob::BlobError>,
        Result<blob::BlobReader, blob::BlobError>,
    ) = tokio::join!(
        cold.execute(ForkInvocation::new(
            cap(0),
            address.clone(),
            authorization(&fixture["ranges"][0])
        )),
        cold.execute(ForkInvocation::new(
            cap(3),
            address.clone(),
            authorization(&fixture["ranges"][1])
        ))
    );
    anyhow::ensure!(
        a.is_ok() != b.is_ok(),
        "expected the documented cold-range one-request rejection"
    );
    let (index, offset, error) = match (a, b) {
        (Err(e), Ok(_)) => (0, 0, e),
        (Ok(_), Err(e)) => (1, 3, e),
        _ => unreachable!(),
    };
    println!("v0.6.15 cold concurrent range failure: {error:?}");
    let mut recovered: blob::BlobReader = cold
        .execute(ForkInvocation::new(
            cap(offset),
            address,
            authorization(&fixture["ranges"][index]),
        ))
        .await?;
    let mut got = Vec::new();
    while let Some(chunk) = recovered.next().await? {
        got.extend(chunk);
    }
    anyhow::ensure!(
        got == bytes[offset as usize..offset as usize + 3],
        "explicit retry did not recover exact range"
    );
    println!("v0.6.15 explicit retry recovered the rejected range");
    println!(
        "v0.6.15 actual provider: sequential range cache401 retry and concurrent distinct archive reads passed; cold concurrent differing ranges reproduce one safe rejection"
    );
    Ok(())
}
