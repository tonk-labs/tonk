//! `tonk eval` seals a command's secret field the way the host does: text
//! bound to an attribute that holds a sealed secret is written as bytes
//! sealed to this site's operator key, never as the text.

use anyhow::Result;
use dialog_artifacts::Entity;
use dialog_query::{Concept, Output as _, Query, Term};
use tonk_schema::domain::command::current::join::SealedUrl;

use crate::common::TestSite;

/// Anything holding a sealed join url. Durable here, so what the
/// evaluation wrote can be read back; a command's transient is swept by
/// its commit.
#[derive(Concept, Debug, Clone)]
pub struct Held {
    pub this: Entity,
    pub url: SealedUrl,
}

const URL: &str = "https://tonk.test/join?access=x#secret-seed";

#[dialog_common::test]
async fn eval_seals_a_secret_to_the_sites_operator() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(&format!(
        r#"
concept!: &held/url
  description: "holds a sealed url"
  with:
    url:
      description: "the url, sealed"
      the: xyz.tonk.command.join/sealed-url
      as: secret

held/url!:
  this: id:held
  url: "{URL}"
"#
    ))
    .await?;

    let session = test.site.branch().await?;
    let held: Vec<Held> = session
        .handle()
        .query()
        .select(Query::<Held> {
            this: Term::var("this"),
            url: Term::var("url"),
        })
        .perform(&test.site.operator)
        .try_vec()
        .await?;
    let [held] = held.as_slice() else {
        anyhow::bail!("expected one sealed url, got {held:?}");
    };
    let bytes = held.url.0.as_bytes();
    assert!(
        !bytes.windows(11).any(|window| window == b"secret-seed"),
        "what was written holds no seed"
    );
    assert_eq!(format!("{:?}", held.url.0), "[sealed]");
    let key = tonk_schema::secret::derive_key(test.site.operator.inner().credential().signer())
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        tonk_schema::secret::reveal(&key, &held.url)?,
        URL,
        "sealed to the key the worker on this profile opens with"
    );
    Ok(())
}
