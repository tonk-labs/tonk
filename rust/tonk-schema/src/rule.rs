//! Rule definitions — deductive and inductive.
//!
//! Re-exports both rule kinds from `dialog_query`:
//!
//! - [`DeductiveRule`] / [`DeductiveRuleDescriptor`] — derive new
//!   facts on demand when their body is queried.
//! - [`InductiveRule`] / [`InductiveRuleDescriptor`] — assert head
//!   facts on commit when their body matches. The tonk surface
//!   keyword is `effect!:`.
//!
//! Both descriptors carry a head (a [`ConceptDescriptor`]) plus
//! `when` and `unless` premises and share the same analysis
//! pipeline. No tonk-flavored wrapper here — the dialog descriptors
//! are already fully `Serialize`/`Deserialize`, so JSON or YAML
//! definitions parse straight into them.
//!
//! [`ConceptDescriptor`]: dialog_query::ConceptDescriptor

pub use dialog_query::{
    DeductiveRule, DeductiveRuleDescriptor, InductiveRule, InductiveRuleDescriptor, Rule,
};
