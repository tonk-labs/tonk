//! [`Params`] — the captured parameter values of a matched [`Route`](crate::Route).

use std::collections::BTreeMap;

/// The named parameter values bound by parsing a URL against a [`Route`], and the
/// input to formatting a URL from a route.
///
/// Insertion-independent and order-independent (a `BTreeMap` underneath) so two
/// `Params` with the same entries compare equal regardless of how they were
/// built — parsing `/x/y` and constructing `{a:x, b:y}` by hand yield equal
/// values, which keeps round-trip assertions honest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params {
    values: BTreeMap<String, String>,
}

impl Params {
    /// An empty parameter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `name` to `value`, returning the (possibly replaced) previous value.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) -> Option<String> {
        self.values.insert(name.into(), value.into())
    }

    /// The value bound to `name`, if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Whether `name` is bound.
    pub fn contains(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// The number of bound parameters.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no parameters are bound.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterate `(name, value)` pairs in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Build a [`Params`] from `(name, value)` pairs — convenient in tests and
/// call sites that already have the bindings in hand.
impl<K, V> FromIterator<(K, V)> for Params
where
    K: Into<String>,
    V: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut params = Params::new();
        for (name, value) in iter {
            params.insert(name, value);
        }
        params
    }
}
