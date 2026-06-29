//! Bidirectional URL routing as a parser-combinator grammar.
//!
//! A [`Route`] is a sequence of literal text and named parameters that both
//! **parses** a URL into captured params and **formats** params back into a URL —
//! one definition, two directions, round-trip by construction
//! (`format(parse(url)) == url`).
//!
//! This is a Rust adaptation of [subroute] (the author's TypeScript type-safe
//! routing library), which draws on [elm/parser] and the type-safe routing of
//! [Spock]. Literals are matched and ignored (elm's `|.`), params are kept (elm's
//! `|=`), and a param consumes greedily up to the next literal (elm's
//! `chompWhile`/`getChompedString`); a param's value type participates in
//! matching, as `subroute`'s typed params do.
//!
//! Two properties make this the right substrate for Tonk routes, where neither
//! `matchit` nor `leptos_router` suffice:
//!
//! - **Intra-segment params.** A pattern like `{entity}@{model}!{view}` binds
//!   three params in one URL segment, split on the literals `@` and `!`. The
//!   literals are just [`Term::Text`] between [`Term::Param`]s — no special case.
//! - **Slash-tolerant params.** A `{model}` that must capture `tonk/person`
//!   (a name containing `/`) works because a param chomps up to the next *fixed*
//!   literal, slashes included — the boundary is the next literal, not `/`.
//!
//! This crate is the engine (the [`Term`]/[`Route`]/[`Params`] core). The string
//! pattern syntax (`"/space/{space}/{entity}@{model}!{view}"`) compiles to a
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
