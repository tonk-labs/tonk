//! Wire-form query bodies derived from the concepts they select.
//!
//! A subscription body written by hand drifts from the shape the worker
//! writes, and drifts silently: a wrong attribute name or a `this` that
//! does not match matches nothing, and a caller sees an answer that
//! never arrives rather than an error. Deriving from the concept is what
//! keeps the reader and the writer honest about the same fact.

use dialog_query::{ConceptQuery, Query as ConceptPattern};
use tonk_schema::EmailStatus;
use tonk_schema::query::Query as WireQuery;

/// Wire-form body selecting the email-status overlay row.
///
/// Read by the account panel's entry, the share dialog's registration
/// form, and the tests that ask what an address answered.
pub fn email_status_wire_query() -> serde_json::Value {
    let pattern = ConceptPattern::<EmailStatus>::default();
    let query = ConceptQuery::from(pattern);
    let wire = WireQuery::from(&query);
    serde_json::to_value(&wire).expect("EmailStatus query serializes")
}
