//! Profile identity concepts: the durable display-name override stored on
//! the profile meta branch. (The transient self overlay used by the
//! topbar chip is added with the chip UI.)

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::profile::DisplayName;

/// The profile's chosen display name, stored on the profile meta branch
/// keyed by the profile DID entity. Cardinality-one: a rename overwrites
/// in place. Absent until the user renames — `tonk-worker` resolves the
/// effective name as "this override, else `petname(did)`".
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileName {
    /// The profile DID entity the name attaches to.
    pub this: Entity,
    /// The chosen display name.
    pub name: DisplayName,
}

impl ProfileName {
    /// A display-name override for the given profile entity.
    pub fn new(profile: Entity, name: String) -> Self {
        Self {
            this: profile,
            name: DisplayName(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProfileName;
    use crate::prelude::EntityExt;
    use dialog_artifacts::Entity;

    #[test]
    fn it_attaches_the_name_to_the_profile_entity() {
        let entity = Entity::of(&"profile-x");
        let pn = ProfileName::new(entity.clone(), "fancy-otter".into());
        assert_eq!(pn.this, entity);
        assert_eq!(pn.name.0, "fancy-otter");
    }
}
