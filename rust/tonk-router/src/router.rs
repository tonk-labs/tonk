//! [`Router`] — a table of [`Route`]s matched against a URL, most-specific-first.
//!
//! A [`Route`] matches one pattern; a [`Router`] holds many and picks the best
//! match for a URL. This is the `oneOf` of the combinator lineage (elm/parser,
//! subroute): try candidates in order, keep the first that parses. The order is
//! **specificity** (static beats param beats catch-all), so it is independent of
//! insertion order — adding a `/{model}` route never shadows a more specific
//! `/board`, regardless of which was registered first.
//!
//! Each route is paired with a caller value `V` (e.g. the route's entity + its
//! model concept) returned on a match alongside the captured [`Params`].

use crate::params::Params;
use crate::route::ParseError;
use crate::route::Route;
use crate::term::{Kind, Term};

/// A routing table: ordered [`Route`]s, each carrying a value of type `V`.
///
/// Build with [`Router::new`] then [`Router::insert`] (or collect via
/// [`FromIterator`]); match with [`Router::recognize`]. Routes are kept sorted by
/// descending specificity so [`recognize`](Router::recognize) tries the most
/// specific first.
#[derive(Clone, Debug)]
pub struct Router<V> {
    /// `(route, value)` entries, sorted by descending specificity.
    entries: Vec<(Route, V)>,
}

/// A successful match: the value of the matched route and the captured params.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match<'a, V> {
    /// The matched route's caller value.
    pub value: &'a V,
    /// The params captured from the URL.
    pub params: Params,
}

/// Why a URL matched no route in the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoMatch {
    /// The table is empty — there is nothing to match against.
    Empty,
    /// No route matched. `furthest` is the parse error of the route that got
    /// closest (highest [`ParseError::at`]) — the most useful near-miss to report.
    NoRoute {
        /// The furthest-progress parse error across all candidates.
        furthest: ParseError,
    },
}

impl<V> Router<V> {
    /// An empty table.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a route and its value, keeping the table sorted by descending
    /// specificity (see [`specificity`]). Ties (equal specificity) preserve
    /// insertion order, so a caller can still impose a deterministic tiebreak by
    /// inserting in a stable order (e.g. by entity URI).
    pub fn insert(&mut self, route: Route, value: V) {
        let score = specificity(&route);
        // Find the first existing entry strictly less specific and insert before
        // it; a stable insert keeps insertion order among equal scores.
        let position = self
            .entries
            .iter()
            .position(|(existing, _)| specificity(existing) < score)
            .unwrap_or(self.entries.len());
        self.entries.insert(position, (route, value));
    }

    /// Match `url` against the table, most-specific-first, returning the first
    /// route that parses together with its captured params. When nothing matches,
    /// reports the furthest-progress near-miss (or [`NoMatch::Empty`]).
    pub fn recognize(&self, url: &str) -> Result<Match<'_, V>, NoMatch> {
        if self.entries.is_empty() {
            return Err(NoMatch::Empty);
        }
        let mut furthest: Option<ParseError> = None;
        for (route, value) in &self.entries {
            match route.parse(url) {
                Ok(params) => return Ok(Match { value, params }),
                Err(error) => {
                    if furthest.as_ref().is_none_or(|best| error.at() >= best.at()) {
                        furthest = Some(error);
                    }
                }
            }
        }
        Err(NoMatch::NoRoute {
            furthest: furthest.expect("non-empty table yields at least one error"),
        })
    }

    /// The routes in the table, most-specific-first.
    pub fn routes(&self) -> impl Iterator<Item = (&Route, &V)> {
        self.entries.iter().map(|(route, value)| (route, value))
    }
}

impl<V> Default for Router<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> FromIterator<(Route, V)> for Router<V> {
    fn from_iter<I: IntoIterator<Item = (Route, V)>>(iter: I) -> Self {
        let mut router = Router::new();
        for (route, value) in iter {
            router.insert(route, value);
        }
        router
    }
}

/// A route's specificity: higher = more specific = tried first.
///
/// The ranking is lexicographic, encoded into one integer so sorting is a plain
/// comparison:
///
/// 1. **Fewer broad params win.** A [`Kind::Rest`] catch-all is the least
///    specific, then [`Kind::Path`] (slash-tolerant), then [`Kind::Segment`].
///    The broadest extent any param uses dominates: a route with a `Rest` param
///    is always less specific than one without.
/// 2. **More literal text wins.** Among routes of the same broadest-extent,
///    the one matching more fixed characters is more specific (`/board` beats
///    `/{x}`).
/// 3. **Fewer params win.** A final tiebreak: fewer captures = more constrained.
fn specificity(route: &Route) -> u32 {
    let mut broadest = 0u32; // 0 = only literals/Segment; higher = broader extent
    let mut literal_len = 0u32;
    let mut params = 0u32;
    for term in route.terms() {
        match term {
            Term::Text(text) => literal_len += text.len() as u32,
            Term::Param { kind, .. } => {
                params += 1;
                let breadth = match kind {
                    Kind::Segment => 1,
                    Kind::Path => 2,
                    Kind::Rest => 3,
                };
                broadest = broadest.max(breadth);
            }
        }
    }
    // Lower `broadest` is MORE specific, so invert it into the high bits; then
    // more literal text; then fewer params (invert). Clamp the sub-scores so they
    // pack without overflow for any realistic route.
    let breadth_score = (3 - broadest.min(3)) << 24; // 0..=3 → high bits, inverted
    let literal_score = literal_len.min(0xFFFF) << 8; // middle bits
    let param_score = (0xFF - params.min(0xFF)) & 0xFF; // low bits, inverted
    breadth_score | literal_score | param_score
}
