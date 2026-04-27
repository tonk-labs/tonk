//! Inductive rule definitions.
//!
//! Re-exports [`DeductiveRuleDescriptor`] from `dialog_query`.
//! Rules carry a `deduce` head (a [`ConceptDescriptor`]) plus
//! `when` and `unless` premises; once compiled with
//! [`DeductiveRuleDescriptor::compile`] they participate in query
//! planning like any built-in concept.
//!
//! No tonk-flavored wrapper here — the dialog descriptor is already
//! fully `Serialize`/`Deserialize`, so JSON or YAML rule definitions
//! parse straight into it.
//!
//! [`ConceptDescriptor`]: dialog_query::ConceptDescriptor

pub use dialog_query::DeductiveRuleDescriptor;
