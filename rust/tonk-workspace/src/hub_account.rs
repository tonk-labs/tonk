//! Pure routing decisions for the Hub account control.

/// Return a navigation destination for the account trigger, or `None` when
/// the trigger should open the local profile roster in place.
pub(crate) fn account_trigger_destination(active_provider: Option<&str>) -> Option<&'static str> {
    (active_provider == Some("false")).then_some("/account")
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_routes_a_provider_free_profile_to_account_setup() {
        assert_eq!(
            super::account_trigger_destination(Some("false")),
            Some("/account")
        );
    }

    #[test]
    fn it_keeps_the_profile_roster_for_an_attached_or_loading_profile() {
        assert_eq!(super::account_trigger_destination(Some("true")), None);
        assert_eq!(super::account_trigger_destination(None), None);
    }
}
