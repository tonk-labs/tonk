//! Compile a pattern string into a [`Route`].
//!
//! The pattern syntax mirrors matchit: `{name}` is a single-segment param,
//! `{*name}` is a slash-tolerant multi-segment span, everything else is literal
//! text. `/space/{space}/{*entity}@{*model}!{*view}` compiles to alternating
//! [`Term::Text`]/[`Term::Param`] terms — the `@` and `!` between params are just
//! literal text, which is how intra-segment params fall out for free.
//!
//! The one extension over matchit: matchit allows a single trailing `{*name}`
//! catch-all; here SEVERAL spans may appear in one route with literals between
//! them (the `{*entity}@{*model}!{*view}` shape).
//!
//! Params:
//! - `{name}` — [`Kind::Segment`]: one segment, stops at `/`.
//! - `{*name}` — [`Kind::Span`]: slash-tolerant, up to the next literal (or end).

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
    /// A param had an empty name (`{}` or `{*}`).
    EmptyName {
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

        Ok(Route::new(terms))
    }
}

/// Parse a param hole body into its name and kind. A leading `*` marks a
/// slash-tolerant span ([`Kind::Span`]); otherwise a single segment.
fn parse_param_body(body: &str, open: usize) -> Result<(String, Kind), PatternError> {
    let (name, kind) = match body.strip_prefix('*') {
        Some(rest) => (rest, Kind::Span),
        None => (body, Kind::Segment),
    };
    if name.is_empty() {
        return Err(PatternError::EmptyName { at: open });
    }
    Ok((name.to_owned(), kind))
}
