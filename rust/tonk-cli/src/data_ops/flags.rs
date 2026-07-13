//! Build a `clap::Command` from a concept's schema so `tonk assert
//! <concept>` gets real typed `--flags`, `--help`, and enumerating
//! errors — all driven by the branch schema, not hand-rolled.

use dialog_query::{Cardinality, ConceptDescriptor};

use crate::schema::type_to_notation;

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
pub fn parse_field_flags(
    descriptor: &ConceptDescriptor,
    concept: &str,
    argv: &[String],
    all_required: bool,
) -> Result<Vec<(String, String)>, clap::Error> {
    let mut cmd = clap::Command::new(format!("tonk … {concept}")).no_binary_name(true);
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
    let matches = cmd.try_get_matches_from(argv)?;
    Ok(field_names
        .into_iter()
        .filter_map(|field| {
            matches
                .get_one::<String>(&field)
                .map(|value| (field.clone(), value.clone()))
        })
        .collect())
}
