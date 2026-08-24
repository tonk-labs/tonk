//! Build a `clap::Command` from a concept's schema so `tonk assert
//! <concept>` gets real typed `--flags`, `--help`, and enumerating
//! errors — all driven by the branch schema, not hand-rolled.

use dialog_query::{Cardinality, ConceptDescriptor};

use crate::data_ops::WriteOptions;
use crate::schema::type_to_notation;

/// The switches every write verb shares, spelled as this dynamic command
/// sees them.
///
/// `assert` receives everything after `<CONCEPT>` raw, so these cannot be
/// declared on the static subcommand — they are built into the concept's own
/// command instead, which also puts them in `tonk assert <concept> --help`
/// beside the fields.
const WRITE_SWITCHES: [(&str, char, &str); 3] = [
    (
        "dry-run",
        '\0',
        "Analyze and plan the write, then drop it instead of committing",
    ),
    (
        "no-sync",
        '\0',
        "Skip the automatic pull-before / push-after",
    ),
    ("quiet", 'q', "Print the envelope without the matched rows"),
];

/// What a concept's dynamic command yielded.
pub struct ParsedFields {
    /// The `(field, value)` pairs actually supplied, in schema field order.
    pub pairs: Vec<(String, String)>,
    /// The shared write switches.
    pub write: WriteOptions,
}

/// Parse schema-derived `--field value` flags out of `argv` against
/// `concept`'s descriptor. With `all_required`, every field becomes
/// a required arg (the mint form of `assert`, where every
/// non-optional field must be supplied up front); otherwise every
/// field is optional (the supersede form of `assert`, where a
/// partial update is the point). Returns the `(field, value)` pairs
/// the caller actually supplied, in schema field order.
///
/// A `--help` (or `-h`) anywhere in `argv` surfaces as `Err(e)`
/// with `e.kind() == clap::error::ErrorKind::DisplayHelp`; `e` then
/// renders the concept's dynamic help. Any other parse failure
/// (unknown flag, missing required flag, …) is also `Err(e)`, and
/// `e`'s rendered usage line enumerates the concept's real flags.
///
/// [`WRITE_SWITCHES`] are added alongside the fields, except where a
/// concept already defines a field of that name — the schema wins, because
/// a field is the only one of the two that cannot be spelled another way.
pub fn parse_field_flags(
    descriptor: &ConceptDescriptor,
    concept: &str,
    argv: &[String],
    all_required: bool,
) -> Result<ParsedFields, clap::Error> {
    // The usage line clap renders is the one an agent reads on every
    // mis-shaped write, so it has to be a command that can be copied.
    // `all_required` is the mint form (no entity); the supersede form
    // takes the entity between the concept and the flags.
    let invocation = if all_required {
        format!("tonk assert {concept}")
    } else {
        format!("tonk assert {concept} <ENTITY>")
    };
    let mut cmd = clap::Command::new(invocation.clone())
        .bin_name(invocation)
        .no_binary_name(true);
    if let Some(about) = descriptor.description() {
        cmd = cmd.about(about.to_string());
    }
    let field_names: Vec<String> = descriptor
        .with()
        .iter()
        .map(|(field, _)| field.to_string())
        .collect();
    for (field, fd) in descriptor.with().iter() {
        let ty = fd
            .content_type()
            .map(|t| type_to_notation(&t))
            .unwrap_or_else(|| "value".into());
        let mut help = fd.description().to_string();
        if matches!(fd.cardinality(), Cardinality::Many) {
            help.push_str(" (cardinality many: each assert appends a value)");
        }
        cmd = cmd.arg(
            clap::Arg::new(field.to_string())
                .long(field.to_string())
                .value_name(ty.to_uppercase())
                .help(help)
                .required(all_required && !fd.is_optional()),
        );
    }
    let mut switches = Vec::new();
    for (long, short, help) in WRITE_SWITCHES {
        if field_names.iter().any(|field| field == long) {
            continue;
        }
        let mut arg = clap::Arg::new(long)
            .long(long)
            .help(help)
            .action(clap::ArgAction::SetTrue);
        if short != '\0' {
            arg = arg.short(short);
        }
        cmd = cmd.arg(arg);
        switches.push(long);
    }

    let matches = cmd.try_get_matches_from(argv)?;
    let flag = |name: &str| switches.contains(&name) && matches.get_flag(name);
    Ok(ParsedFields {
        pairs: field_names
            .iter()
            .filter_map(|field| {
                matches
                    .get_one::<String>(field)
                    .map(|value| (field.clone(), value.clone()))
            })
            .collect(),
        write: WriteOptions {
            dry_run: flag("dry-run"),
            no_sync: flag("no-sync"),
            quiet: flag("quiet"),
        },
    })
}
