//! [`Route`] — an ordered list of [`Term`]s that parses URLs into [`Params`] and
//! formats [`Params`] back into URLs.

use crate::params::Params;
use crate::term::{Kind, Term};
use crate::value::Type;

/// A bidirectional route: a sequence of literals and named params.
///
/// Build one from [`Term`]s ([`Route::new`]) or compile it from a pattern string
/// ([`Route::parse_pattern`]). [`Route::parse`] turns a URL into [`Params`];
/// [`Route::format`] turns [`Params`] back into a URL. They are inverses: for any
/// URL the route matches, `format(parse(url)) == url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    terms: Vec<Term>,
}

/// Why a URL failed to parse against a [`Route`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// A literal [`Term::Text`] was expected at `at` but the input there did not
    /// start with `expected`. `at` is a byte offset into the original input.
    Expected {
        /// The literal text the route required.
        expected: String,
        /// Byte offset into the input where it was required.
        at: usize,
    },
    /// A [`Term::Param`] captured nothing — a param may not be empty (an empty
    /// capture is almost always a mis-parse, e.g. `//` or a missing id).
    EmptyParam {
        /// The name of the param that captured an empty span.
        name: String,
        /// Byte offset into the input where the param began.
        at: usize,
    },
    /// A [`Term::Param`] captured a value its [`Type`](crate::Type) rejected
    /// (e.g. a non-numeric capture for an `unsigned` param). This is the seam that
    /// lets two structurally identical routes be told apart by param type: the
    /// table tries the next candidate route on this error.
    InvalidType {
        /// The param name.
        name: String,
        /// The value type's name that rejected the value.
        ty: String,
        /// The offending captured value.
        value: String,
        /// Byte offset into the input where the param began.
        at: usize,
    },
    /// The route was fully consumed but input remained (the URL is longer than
    /// the pattern). `at` is where the leftover begins.
    Trailing {
        /// Byte offset into the input where the unmatched remainder begins.
        at: usize,
    },
}

impl ParseError {
    /// The byte offset into the input where parsing failed — how *far* this route
    /// got. The routing table uses the furthest-progress error to report the
    /// most relevant near-miss when nothing matches.
    pub fn at(&self) -> usize {
        match self {
            ParseError::Expected { at, .. }
            | ParseError::EmptyParam { at, .. }
            | ParseError::InvalidType { at, .. }
            | ParseError::Trailing { at } => *at,
        }
    }
}

/// Why [`Params`] failed to format against a [`Route`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// The route has a [`Term::Param`] named `name` but the supplied [`Params`]
    /// did not bind it.
    Missing {
        /// The unbound param name.
        name: String,
    },
    /// A supplied value contains a character its param's [`Kind`] forbids (e.g. a
    /// `/` in a [`Kind::Segment`] value), so formatting then parsing would not
    /// round-trip. Rejected rather than silently producing an unparseable URL.
    Invalid {
        /// The param name.
        name: String,
        /// The offending value.
        value: String,
    },
}

impl Route {
    /// A route from an explicit term list.
    pub fn new(terms: impl Into<Vec<Term>>) -> Self {
        Self {
            terms: terms.into(),
        }
    }

    /// The route's terms, in order.
    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    /// Assign value [`Type`]s to params by name — the binding-layer seam.
    ///
    /// A pattern string carries only *extent* (`{name}` / `{*name}`), never a
    /// value type: a param's type comes from the route *model field* it fills
    /// (`as: entity` / `as: unsigned` / …), not the URL.
    /// After [`parse_pattern`] compiles a route with all params defaulted to
    /// [`text`](crate::value::text), the binding layer calls this with a
    /// `name -> Type` lookup (built from the model's field descriptors) to install
    /// the real types, so they then participate in matching. Params absent from
    /// `types` keep their current type. Returns `self` for chaining.
    ///
    /// [`parse_pattern`]: Route::parse_pattern
    /// [`Type`]: crate::Type
    pub fn with_types(mut self, types: impl Fn(&str) -> Option<Type>) -> Self {
        for term in &mut self.terms {
            if let Term::Param { name, ty, .. } = term
                && let Some(replacement) = types(name)
            {
                *ty = replacement;
            }
        }
        self
    }

    /// Parse `input` against this route, binding each param.
    ///
    /// Walks the terms left to right: a [`Term::Text`] must match verbatim; a
    /// [`Term::Param`] captures from the cursor up to wherever the *next* literal
    /// begins (or end of input), restricted to the characters its [`Kind`]
    /// admits. The whole input must be consumed — a leftover tail is a
    /// [`ParseError::Trailing`], so a route never matches a longer URL by prefix.
    pub fn parse(&self, input: &str) -> Result<Params, ParseError> {
        let mut params = Params::new();
        let mut cursor = 0usize;

        for (index, term) in self.terms.iter().enumerate() {
            match term {
                Term::Text(literal) => {
                    if input[cursor..].starts_with(literal.as_str()) {
                        cursor += literal.len();
                    } else {
                        return Err(ParseError::Expected {
                            expected: literal.clone(),
                            at: cursor,
                        });
                    }
                }
                Term::Param { name, kind, ty } => {
                    let start = cursor;
                    let end = self.param_end(input, start, *kind, index)?;
                    if end == start {
                        return Err(ParseError::EmptyParam {
                            name: name.clone(),
                            at: start,
                        });
                    }
                    let value = &input[start..end];
                    // Validate through the param's value type. A rejection here is
                    // what lets two structurally identical routes be told apart by
                    // type — the table moves on to the next candidate.
                    if !ty.validate(value) {
                        return Err(ParseError::InvalidType {
                            name: name.clone(),
                            ty: ty.name().to_owned(),
                            value: value.to_owned(),
                            at: start,
                        });
                    }
                    params.insert(name.clone(), value);
                    cursor = end;
                }
            }
        }

        if cursor == input.len() {
            Ok(params)
        } else {
            Err(ParseError::Trailing { at: cursor })
        }
    }

    /// Format `params` into a URL by emitting each term in order.
    ///
    /// Literals are emitted verbatim; params are looked up by name. A value is
    /// rejected ([`FormatError::Invalid`]) if it contains a character its
    /// [`Kind`] forbids, since that would not round-trip back through [`parse`].
    ///
    /// [`parse`]: Route::parse
    pub fn format(&self, params: &Params) -> Result<String, FormatError> {
        let mut out = String::new();
        for term in &self.terms {
            match term {
                Term::Text(literal) => out.push_str(literal),
                Term::Param { name, kind, ty } => {
                    let value = params
                        .get(name)
                        .ok_or_else(|| FormatError::Missing { name: name.clone() })?;
                    // Reject a value that wouldn't round-trip: empty, containing a
                    // char its extent forbids (a `/` in a Segment), or failing its
                    // value type. Better a format error than an unparseable URL.
                    if value.is_empty()
                        || value.chars().any(|ch| !kind.admits(ch))
                        || !ty.validate(value)
                    {
                        return Err(FormatError::Invalid {
                            name: name.clone(),
                            value: value.to_owned(),
                        });
                    }
                    out.push_str(value);
                }
            }
        }
        Ok(out)
    }

    /// The byte offset where a param starting at `start` ends.
    ///
    /// The capture is bounded by *both* constraints, whichever is tighter:
    ///
    /// 1. The **admissible run** — the longest prefix of remaining input whose
    ///    characters the kind admits. A [`Kind::Segment`] stops at the first `/`;
    ///    a [`Kind::Span`] admits everything.
    /// 2. The **next literal** — if a [`Term::Text`] follows this param, the
    ///    capture ends where that literal first appears.
    ///
    /// With a following literal the literal must appear *within* the admissible
    /// run (a `Segment` followed by `@` requires the `@` before any `/`),
    /// otherwise it is a [`ParseError::Expected`]. With no following literal the
    /// capture is the whole admissible run (a terminal `Span` thus takes the
    /// rest).
    fn param_end(
        &self,
        input: &str,
        start: usize,
        kind: Kind,
        term_index: usize,
    ) -> Result<usize, ParseError> {
        // (1) The admissible run.
        let admissible_end = start
            + input[start..]
                .char_indices()
                .find(|(_, ch)| !kind.admits(*ch))
                .map(|(offset, _)| offset)
                .unwrap_or(input.len() - start);

        // (2) The next literal after this param, if any.
        let next_literal = self.terms[term_index + 1..].iter().find_map(|term| {
            if let Term::Text(literal) = term {
                Some(literal.as_str())
            } else {
                None
            }
        });

        match next_literal {
            // Bounded by the next literal: it must begin *at or before* the end of
            // the admissible run. Search the remaining input (not just the run) so
            // a literal sitting exactly at the run boundary — e.g. the `/` right
            // after a `Segment` value — is found; then require that occurrence to
            // be within the run. A match past the run means the literal can't be
            // reached without crossing a forbidden char (an `@` only after a `/`
            // for a Segment), which is a miss.
            Some(literal) => match input[start..].find(literal) {
                Some(offset) if start + offset <= admissible_end => Ok(start + offset),
                _ => Err(ParseError::Expected {
                    expected: literal.to_owned(),
                    at: admissible_end,
                }),
            },
            // No following literal: take the whole admissible run.
            None => Ok(admissible_end),
        }
    }
}
