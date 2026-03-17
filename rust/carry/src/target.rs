//! Target and field parsing for carry CLI commands.
//!
//! The `<TARGET>` argument determines the kind of query or assertion:
//!
//! - **Contains `.`**: domain target (e.g. `io.gozala.person`). Fields are
//!   qualified as `domain/field`.
//! - **No `.`**: concept target (e.g. `person`). Resolves a concept by
//!   name or bookmark.
//!
//! Fields take two forms:
//!
//! - `name` — include the field in output (projection)
//! - `name=value` — filter/assert the field with a specific value

use anyhow::Result;

// ---------------------------------------------------------------------------
// Target — domain vs concept
// ---------------------------------------------------------------------------

/// A parsed target specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Domain target (contains `.`): fields are qualified as `{domain}/{field}`.
    Domain(String),
    /// Concept target (no `.`): resolved by name or bookmark lookup.
    Concept(String),
}

impl Target {
    /// Parse a target string.
    ///
    /// If it contains a `.` it is a domain target; otherwise a concept target.
    pub fn parse(input: &str) -> Result<Self> {
        if input.is_empty() {
            anyhow::bail!("Target cannot be empty");
        }
        if input.contains('.') {
            Ok(Target::Domain(input.to_string()))
        } else {
            Ok(Target::Concept(input.to_lowercase()))
        }
    }

    /// The namespace used for qualifying field names.
    ///
    /// For domain targets this is the domain string itself (e.g.
    /// `io.gozala.person`). For concept targets this is the lowercased
    /// concept name (used as the attribute namespace prefix).
    pub fn namespace(&self) -> &str {
        match self {
            Target::Domain(d) => d,
            Target::Concept(c) => c,
        }
    }

    /// Whether this is a domain target.
    pub fn is_domain(&self) -> bool {
        matches!(self, Target::Domain(_))
    }

    /// Whether this is a concept target.
    pub fn is_concept(&self) -> bool {
        matches!(self, Target::Concept(_))
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Domain(d) => write!(f, "{}", d),
            Target::Concept(c) => write!(f, "{}", c),
        }
    }
}

// ---------------------------------------------------------------------------
// Field — projection or filter
// ---------------------------------------------------------------------------

/// A parsed field specifier from the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field name as given by the user (may be short or qualified).
    pub name: String,
    /// Optional value (present when the user writes `name=value`).
    pub value: Option<String>,
}

impl Field {
    /// Qualify this field name within a namespace, producing the full
    /// attribute selector (e.g. `io.gozala.person/name`).
    ///
    /// If the name already contains `/` it is returned as-is.
    pub fn qualified_name(&self, namespace: &str) -> String {
        if self.name.contains('/') {
            self.name.clone()
        } else {
            format!("{}/{}", namespace, self.name)
        }
    }

    /// Whether this field is a filter (has a value).
    pub fn is_filter(&self) -> bool {
        self.value.is_some()
    }

    /// Whether this field is a projection (no value, just include in output).
    pub fn is_projection(&self) -> bool {
        self.value.is_none()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a list of field arguments from the CLI.
///
/// Each argument is either:
/// - `name` — projection (include in output)
/// - `name=value` — filter/assertion
///
/// The special field `this=<ENTITY>` is extracted separately and returned
/// as the second element of the tuple.
pub fn parse_fields(args: &[String]) -> Result<(Vec<Field>, Option<String>)> {
    let mut fields = Vec::new();
    let mut this_entity = None;

    for arg in args {
        if let Some(entity) = arg.strip_prefix("this=") {
            if this_entity.is_some() {
                anyhow::bail!("Duplicate `this=` argument");
            }
            this_entity = Some(entity.to_string());
            continue;
        }

        let field = if let Some((name, value)) = arg.split_once('=') {
            Field {
                name: name.to_string(),
                value: Some(value.to_string()),
            }
        } else {
            Field {
                name: arg.to_string(),
                value: None,
            }
        };

        if field.name.is_empty() {
            anyhow::bail!("Field name cannot be empty in '{}'", arg);
        }
        fields.push(field);
    }

    Ok((fields, this_entity))
}

/// Determine whether the first CLI argument is a file path, stdin marker,
/// or a target.
///
/// Used by `assert` and `retract` to disambiguate `<TARGET>` from `<FILE>`.
///
/// Rules from the spec:
/// - `-` → stdin
/// - Contains `/` or ends in `.yaml`, `.yml`, `.json` → file
/// - Otherwise → target
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstArg {
    Stdin,
    File(String),
    Target(Target),
}

impl FirstArg {
    pub fn parse(input: &str) -> Result<Self> {
        if input == "-" {
            return Ok(FirstArg::Stdin);
        }
        if input.contains('/')
            || input.ends_with(".yaml")
            || input.ends_with(".yml")
            || input.ends_with(".json")
        {
            return Ok(FirstArg::File(input.to_string()));
        }
        Ok(FirstArg::Target(Target::parse(input)?))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_target() {
        let t = Target::parse("io.gozala.person").unwrap();
        assert_eq!(t, Target::Domain("io.gozala.person".to_string()));
        assert!(t.is_domain());
        assert_eq!(t.namespace(), "io.gozala.person");
    }

    #[test]
    fn test_concept_target() {
        let t = Target::parse("person").unwrap();
        assert_eq!(t, Target::Concept("person".to_string()));
        assert!(t.is_concept());
        assert_eq!(t.namespace(), "person");
    }

    #[test]
    fn test_concept_lowercased() {
        let t = Target::parse("Person").unwrap();
        assert_eq!(t, Target::Concept("person".to_string()));
    }

    #[test]
    fn test_empty_target_fails() {
        assert!(Target::parse("").is_err());
    }

    #[test]
    fn test_parse_fields_projection() {
        let args = vec!["name".to_string(), "age".to_string()];
        let (fields, this_ent) = parse_fields(&args).unwrap();
        assert!(this_ent.is_none());
        assert_eq!(fields.len(), 2);
        assert!(fields[0].is_projection());
        assert!(fields[1].is_projection());
    }

    #[test]
    fn test_parse_fields_filter() {
        let args = vec!["name=Alice".to_string(), "age".to_string()];
        let (fields, _) = parse_fields(&args).unwrap();
        assert_eq!(fields.len(), 2);
        assert!(fields[0].is_filter());
        assert_eq!(fields[0].value.as_deref(), Some("Alice"));
        assert!(fields[1].is_projection());
    }

    #[test]
    fn test_parse_fields_this() {
        let args = vec!["this=did:key:z123".to_string(), "name=Alice".to_string()];
        let (fields, this_ent) = parse_fields(&args).unwrap();
        assert_eq!(this_ent, Some("did:key:z123".to_string()));
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn test_duplicate_this_fails() {
        let args = vec!["this=did:key:z1".to_string(), "this=did:key:z2".to_string()];
        assert!(parse_fields(&args).is_err());
    }

    #[test]
    fn test_field_qualified_name() {
        let f = Field {
            name: "name".to_string(),
            value: None,
        };
        assert_eq!(
            f.qualified_name("io.gozala.person"),
            "io.gozala.person/name"
        );
    }

    #[test]
    fn test_field_already_qualified() {
        let f = Field {
            name: "other.ns/name".to_string(),
            value: None,
        };
        assert_eq!(f.qualified_name("io.gozala.person"), "other.ns/name");
    }

    #[test]
    fn test_first_arg_stdin() {
        assert_eq!(FirstArg::parse("-").unwrap(), FirstArg::Stdin);
    }

    #[test]
    fn test_first_arg_file() {
        assert_eq!(
            FirstArg::parse("data.yaml").unwrap(),
            FirstArg::File("data.yaml".to_string())
        );
        assert_eq!(
            FirstArg::parse("path/to/file").unwrap(),
            FirstArg::File("path/to/file".to_string())
        );
        assert_eq!(
            FirstArg::parse("data.json").unwrap(),
            FirstArg::File("data.json".to_string())
        );
    }

    #[test]
    fn test_first_arg_target() {
        match FirstArg::parse("person").unwrap() {
            FirstArg::Target(Target::Concept(c)) => assert_eq!(c, "person"),
            _ => panic!("Expected concept target"),
        }
        match FirstArg::parse("io.gozala.person").unwrap() {
            FirstArg::Target(Target::Domain(d)) => assert_eq!(d, "io.gozala.person"),
            _ => panic!("Expected domain target"),
        }
    }
}
