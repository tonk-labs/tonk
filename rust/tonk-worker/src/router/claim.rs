//! A claim whose attribute is only known at runtime.

use dialog_artifacts::{Attribute, Entity, Statement, Update, Value};

/// This implements `Statement` by forwarding to `Update::associate`/`dissociate`,
/// allowing us to use runtime-determined attribute names and `Value` directly.
/// Used by the evaluate and blob paths and by `session::stamp_site` to write the
/// route's captured params as `xyz.tonk.site/{name}` facts whose names aren't
/// known until match time.
///
/// `unique` selects the assert cardinality: `false` → [`Update::associate`]
/// (cardinality-many); `true` →
/// [`Update::associate_unique`], which emits a replace so a prior value at the
/// same `(entity, attribute)` is superseded across commits — required for the
/// per-tab site params, which must reflect only the latest navigation rather than
/// accumulate one value per visited route.
#[derive(Clone)]
pub(crate) struct RawClaim {
    pub(crate) the: Attribute,
    pub(crate) of: Entity,
    pub(crate) is: Value,
    pub(crate) unique: bool,
}

impl Statement for RawClaim {
    fn assert(self, update: &mut impl Update) {
        if self.unique {
            update.associate_unique(self.the, self.of, self.is);
        } else {
            update.associate(self.the, self.of, self.is);
        }
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}
