//! Utilities for working with dialog-query relations.

/// Validates that a string is a valid attribute (namespace/predicate format).
pub(crate) const fn validate_attribute(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            return true;
        }
        i += 1;
    }
    false
}

/// Creates an Attribute from a string literal with compile-time validation.
macro_rules! the {
    ($s:literal) => {{
        const _: () = {
            if !$crate::relation::validate_attribute($s) {
                panic!(concat!(
                    "invalid attribute: \"",
                    $s,
                    "\" (must be namespace/predicate)"
                ));
            }
        };
        $s.parse::<dialog_query::claim::Attribute>().unwrap()
    }};
}

pub(crate) use the;
