//! The atoms a [`Route`](crate::Route) is made of: literal text and named
//! parameters.

use crate::value::Type;

/// One element of a route: a fixed literal or a captured parameter.
///
/// A [`Route`](crate::Route) is an ordered list of these. Parsing matches the
/// URL against the list left-to-right (literals must appear verbatim, params
/// chomp up to the next literal); formatting emits them in order (literals
/// verbatim, params from the supplied values).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    /// Literal text that must appear verbatim in the URL — a path separator
    /// (`/`), a segment label (`space`), or an intra-segment delimiter (`@`,
    /// `!`). Matched and ignored when parsing (elm/parser's `|.`); emitted
    /// verbatim when formatting.
    Text(String),
    /// A named capture. When parsing, binds the substring up to the next
    /// [`Term::Text`] (or end of input) under `name`, consuming per its
    /// [`Kind`] (extent) and admitted by its [`Type`] (value type); when
    /// formatting, looks `name` up in the supplied [`Params`](crate::Params).
    Param {
        /// The parameter name (`space`, `model`, `entity`, `view`).
        name: String,
        /// How far the param consumes — its extent.
        kind: Kind,
        /// What the captured value must be — its type. The engine validates the
        /// capture *through* this, so two routes can be told apart by param type.
        /// Defaults to [`text`](crate::value::text) (accepts anything); the
        /// binding layer supplies `entity`/`unsigned`/… from the route model.
        ty: Type,
    },
}

impl Term {
    /// A literal term.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// A parameter term with the default ([`text`](crate::value::text)) type.
    pub fn param(name: impl Into<String>, kind: Kind) -> Self {
        Self::Param {
            name: name.into(),
            kind,
            ty: Type::default(),
        }
    }

    /// A parameter term with an explicit value type.
    pub fn typed(name: impl Into<String>, kind: Kind, ty: Type) -> Self {
        Self::Param {
            name: name.into(),
            kind,
            ty,
        }
    }
}

/// What a [`Term::Param`] captures, and thus how far it consumes — the one axis
/// where this router extends matchit.
///
/// A param is always bounded by the *next literal in the route* (or end of
/// input). [`Kind`] decides whether a `/` also bounds it:
///
/// - [`Kind::Segment`] (`{name}`) — a single path segment, like matchit's
///   `{name}`: stops at the next `/`.
/// - [`Kind::Span`] (`{*name}`) — a multi-segment span: `/` does NOT bound it,
///   only the next literal (or end) does. matchit has this as `{*name}` too, but
///   ONLY as a single trailing catch-all; this router allows SEVERAL spans in one
///   route with literals between them (`/{*entity}@{*model}!{*view}`), which is
///   the whole reason the crate exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A single path segment (`{name}`): any characters except `/`.
    Segment,
    /// A slash-tolerant multi-segment span (`{*name}`): any characters up to the
    /// next literal (or end), `/` included. Suits namespaced refs (`tonk/person`)
    /// and URIs (`did:key:…`, `id:foo/bar`). When terminal it captures the rest.
    Span,
}

impl Kind {
    /// Whether `ch` may appear inside a value of this kind.
    pub(crate) fn admits(self, ch: char) -> bool {
        match self {
            Kind::Segment => ch != '/',
            Kind::Span => true,
        }
    }
}
