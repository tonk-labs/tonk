//! Compile a pattern string into a [`Route`].
//!
//! The pattern syntax is the literal grammar a `route!` carries in YAML:
//! `{name}` (or `{name:kind}`) is a parameter, everything else is literal text.
//! `/space/{space}/{entity}@{model}!{view}` compiles to alternating
//! [`Term::Text`]/[`Term::Param`] terms — the `@` and `!` between params are just
//! literal text, which is exactly how intra-segment params fall out for free.
//!
//! Param kinds:
//! - `{name}` — [`Kind::Segment`] (default): one segment, no `/`.
//! - `{name:path}` — [`Kind::Path`]: slash-tolerant (for `tonk/person`).
//! - `{name:rest}` — [`Kind::Rest`]: the entire remaining input (must be last).

use crate::route::Route;
use crate::term::{Kind, Term};

/// Why a pattern string failed to compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternError {
    /// A `{` was opened but never closed before end of input.
    UnclosedParam {
        /// Byte offset of the offending `{`.
        at: usize,
    },
    /// A `}` appeared with no matching `{`.
    UnexpectedClose {
        /// Byte offset of the offending `}`.
        at: usize,
    },
    /// A param had an empty name (`{}` or `{:kind}`).
    EmptyName {
        /// Byte offset of the offending `{`.
        at: usize,
    },
    /// A param named an unknown kind (only `segment`, `path`, `rest`).
    UnknownKind {
        /// The kind token as written.
        kind: String,
        /// Byte offset of the offending `{`.
        at: usize,
    },
    /// A [`Kind::Rest`] param was not the last term — nothing may follow a
    /// catch-all, since it consumes to the end.
    RestNotLast {
        /// Byte offset of the offending `{`.
        at: usize,
    },
}

impl Route {
    /// Compile a pattern string into a [`Route`].
    ///
    /// Scans for `{...}` param holes; the text between holes becomes
    /// [`Term::Text`]. Adjacent literal runs are merged so the term list is
    /// minimal (`/space/` is one literal, not three). See the [module
    /// docs](self) for the syntax.
    pub fn parse_pattern(pattern: &str) -> Result<Route, PatternError> {
        let mut terms: Vec<Term> = Vec::new();
        let mut literal = String::new();
        let bytes = pattern.as_bytes();
        let mut index = 0usize;

        while index < bytes.len() {
            match bytes[index] {
                b'{' => {
                    // Flush any literal accumulated before this param.
                    if !literal.is_empty() {
                        terms.push(Term::Text(std::mem::take(&mut literal)));
                    }
                    let open = index;
                    let close = pattern[index..]
                        .find('}')
                        .map(|offset| index + offset)
                        .ok_or(PatternError::UnclosedParam { at: open })?;
                    let body = &pattern[index + 1..close];
                    let (name, kind) = parse_param_body(body, open)?;
                    terms.push(Term::param(name, kind));
                    index = close + 1;
                }
                b'}' => return Err(PatternError::UnexpectedClose { at: index }),
                _ => {
                    // Advance by one full char (patterns may contain multi-byte
                    // text), appending it to the current literal run.
                    let ch = pattern[index..].chars().next().expect("char at index");
                    literal.push(ch);
                    index += ch.len_utf8();
                }
            }
        }

        if !literal.is_empty() {
            terms.push(Term::Text(literal));
        }

        // A `rest` param must be last — reject a catch-all with anything after it.
        if let Some(position) = terms.iter().position(is_rest_param)
            && position != terms.len() - 1
        {
            return Err(PatternError::RestNotLast {
                // Best-effort offset: the start of the pattern, since the per-term
                // offset is not retained past compilation.
                at: 0,
            });
        }

        Ok(Route::new(terms))
    }
}

/// Parse a param hole body (`name` or `name:kind`) into its name and kind.
fn parse_param_body(body: &str, open: usize) -> Result<(String, Kind), PatternError> {
    let (name, kind) = match body.split_once(':') {
        Some((name, kind)) => (name, parse_kind(kind, open)?),
        None => (body, Kind::Segment),
    };
    if name.is_empty() {
        return Err(PatternError::EmptyName { at: open });
    }
    Ok((name.to_owned(), kind))
}

/// Parse a kind token (`segment`, `path`, `rest`).
fn parse_kind(token: &str, open: usize) -> Result<Kind, PatternError> {
    match token {
        "segment" => Ok(Kind::Segment),
        "path" => Ok(Kind::Path),
        "rest" => Ok(Kind::Rest),
        _ => Err(PatternError::UnknownKind {
            kind: token.to_owned(),
            at: open,
        }),
    }
}

/// Whether a term is a [`Kind::Rest`] param.
fn is_rest_param(term: &Term) -> bool {
    matches!(
        term,
        Term::Param {
            kind: Kind::Rest,
            ..
        }
    )
}
