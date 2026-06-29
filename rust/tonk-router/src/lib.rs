//! A small extension over `matchit`: the same `{name}` single-segment params and
//! static literals, plus **multiple `{*name}` multi-segment spans in one route**
//! with literals between them. A [`Route`] is also bidirectional — it both
//! **parses** a URL into captured params and **formats** params back into a URL,
//! round-trip by construction (`format(parse(url)) == url`).
//!
//! `matchit` already has `{name}` (one segment) and `{*name}` (a multi-segment
//! catch-all), but the catch-all may appear only ONCE, at the end. Tonk routes
//! need more:
//!
//! - **Intra-segment params.** `{*entity}@{*model}!{*view}` binds three spans in
//!   one URL segment, split on the literals `@` and `!` — just [`Term::Text`]
//!   between [`Term::Param`]s.
//! - **Slash-tolerant refs anywhere.** `{*model}` capturing `tonk/person` (a
//!   namespaced ref) works because a span's boundary is the next literal, not
//!   `/` — and any number of spans may appear in one route.
//!
//! So this crate keeps matchit's shape and adds the one capability it lacks. The
//! combinator/round-trip machinery (and the per-param value type that participates
//! in matching) draws on [subroute], [elm/parser], and the type-safe routing of
//! [Spock].
//!
//! This crate is the engine (the [`Term`]/[`Route`]/[`Params`] core). The string
//! pattern syntax (`"/space/{space}/{*entity}@{*model}!{*view}"`) compiles to a
//! [`Route`] via [`Route::parse_pattern`]; the routing *table* (many routes,
//! specificity ordering, match-the-furthest errors) layers on top.
//!
//! [subroute]: https://github.com/Gozala/subroute
//! [elm/parser]: https://package.elm-lang.org/packages/elm/parser/latest/
//! [Spock]: https://www.spock.li/2015/04/19/type-safe_routing.html

mod params;
mod pattern;
mod route;
mod router;
mod term;
pub mod value;

pub use params::Params;
pub use pattern::PatternError;
pub use route::{FormatError, ParseError, Route};
pub use router::{Match, NoMatch, Router};
pub use term::{Kind, Term};
pub use value::{Type, ValueType};
