//! Pure routing decisions for the Hub account control.

/// Whether the account trigger should ask for the registration cluster
/// rather than open the local profile roster.
///
/// A profile with no provider has no roster worth showing and no account
/// to switch between; the only thing to do is link one. It used to
/// navigate to `/settings`, which put two pages between the label and
/// the ceremony — press "log in", land on a panel, press "link an
/// account" there, and only then meet the cluster. The cluster is the
/// whole flow, so the label raises it directly.
pub(crate) fn trigger_asks_to_link(active_provider: Option<&str>) -> bool {
    active_provider == Some("false")
}

/// What the trigger reads.
///
/// The same words the ceremony uses, because it is the same act: the
/// address decides whether it creates a passkey or signs you in, so
/// naming it "log in" told half the users the wrong thing.
pub(crate) fn trigger_label(active_provider: Option<&str>) -> &'static str {
    if trigger_asks_to_link(active_provider) {
        "link an account"
    } else {
        "account"
    }
}

/// What the trigger reads while this device is linking the account.
///
/// A separate word from both other states: the person is past the door
/// (so not "link an account") and the account is not here yet (so not its
/// name). Login finishes when custody is recovered, which makes this
/// window real rather than theoretical.
pub(crate) const LINKING_LABEL: &str = "linking account";

#[cfg(test)]
mod tests {
    /// The three states the cell can be in are three different words.
    ///
    /// The bug this pins: a profile mid-link has no provider yet, so the
    /// roster's answer is "link an account" — offering a door the person
    /// already walked through, over a stack with nothing in it. Linking
    /// has to be its own word, distinct from both the offer and the
    /// account's name.
    #[test]
    fn it_names_linking_apart_from_the_offer_and_the_account() {
        assert_eq!(super::LINKING_LABEL, "linking account");
        assert_ne!(
            super::LINKING_LABEL,
            super::trigger_label(Some("false")),
            "a link in flight must not read as an offer to start one"
        );
        assert_ne!(
            super::LINKING_LABEL,
            super::trigger_label(None),
            "nor as a settled account"
        );
    }

    #[test]
    fn it_asks_a_provider_free_profile_to_link() {
        assert!(super::trigger_asks_to_link(Some("false")));
        assert_eq!(super::trigger_label(Some("false")), "link an account");
    }

    #[test]
    fn it_keeps_the_profile_roster_for_an_attached_or_loading_profile() {
        assert!(!super::trigger_asks_to_link(Some("true")));
        assert!(!super::trigger_asks_to_link(None));
    }
}
