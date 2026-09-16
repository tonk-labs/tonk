use super::*;
use crate::delivery::chunks::{self, Chunk, Kind};
pub(super) async fn publish(
    db: &D1Database,
    first: worker::d1::D1PreparedStatement,
    kind: Kind,
    key: &str,
    marker: &str,
    payload: &str,
) -> Result<bool, StoreError> {
    let mut statements = vec![first];
    for (ordinal, piece) in chunks::pieces(payload).enumerate() {
        statements.push(
            db.prepare(kind.insert())
                .bind(&[
                    JsValue::from_str(key),
                    JsValue::from_f64(ordinal as f64),
                    JsValue::from_str(piece),
                    JsValue::from_str(marker),
                ])
                .map_err(map_err)?,
        );
    }
    let results = db.batch(statements).await.map_err(map_err)?;
    Ok(results.first().map(changed_rows).unwrap_or_default() > 0)
}
pub(super) async fn load(
    db: &D1Database,
    kind: Kind,
    key: &str,
    value: String,
) -> Result<String, StoreError> {
    if !chunks::is_chunked(&value) {
        return Ok(value);
    }
    let parts = db
        .prepare(kind.select())
        .bind(&[JsValue::from_str(key)])
        .map_err(map_err)?
        .all()
        .await
        .map_err(map_err)?
        .results::<Chunk>()
        .map_err(map_err)?;
    chunks::assemble(&value, parts)
}
