#[cfg(target_arch = "wasm32")]
mod inner {}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use carry::help;
    use clap::{Parser, Subcommand};

    #[derive(Parser)]
    #[command(name = "carry")]
    #[command(about = "CLI for Dialog DB - a local-first semantic database for structured data")]
    #[command(long_about = help::MAIN_LONG_ABOUT)]
    #[command(after_help = help::MAIN_AFTER_HELP)]
    pub struct Cli {
        #[command(subcommand)]
        pub command: Commands,

        /// Path to a specific .carry/ repository (skips filesystem search from $PWD)
        #[arg(long, global = true)]
        pub repo: Option<String>,

        /// Target a specific space by DID or label (overrides active space)
        #[arg(long, global = true, hide = true)]
        pub space: Option<String>,

        /// Output format for query results
        #[arg(long, global = true, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
        pub format: String,
    }

    #[derive(Subcommand)]
    pub enum Commands {
        /// Create a new .carry/ repository
        #[command(alias = "i")]
        #[command(long_about = help::INIT_LONG_ABOUT)]
        #[command(after_help = help::INIT_AFTER_HELP)]
        Init {
            /// Label for the repository (stored as a name claim)
            #[arg(value_name = "LABEL")]
            name: Option<String>,
        },

        /// Query entities by domain or concept
        #[command(alias = "q")]
        #[command(long_about = help::QUERY_LONG_ABOUT)]
        #[command(after_help = help::QUERY_AFTER_HELP)]
        Query {
            /// Domain (contains '.') or concept name (no '.') to query
            #[arg(value_name = "TARGET")]
            target: String,

            /// Fields to output or filter. Use 'field' to include in output,
            /// 'field=value' to filter results.
            #[arg(value_name = "FIELD[=VALUE]")]
            fields: Vec<String>,
        },

        /// Assert claims on entities
        #[command(alias = "a")]
        #[command(long_about = help::ASSERT_LONG_ABOUT)]
        #[command(after_help = help::ASSERT_AFTER_HELP)]
        Assert {
            /// Domain/concept, file path (.yaml/.yml/.json), or '-' for stdin
            #[arg(value_name = "TARGET|FILE|-")]
            target_or_file: String,

            /// Claims to assert. Use 'this=<DID>' to target existing entity,
            /// otherwise a new entity is created.
            #[arg(value_name = "FIELD=VALUE")]
            fields: Vec<String>,
        },

        /// Retract claims from entities
        #[command(alias = "r")]
        #[command(long_about = help::RETRACT_LONG_ABOUT)]
        #[command(after_help = help::RETRACT_AFTER_HELP)]
        Retract {
            /// Domain/concept, file path (.yaml/.yml/.json), or '-' for stdin
            #[arg(value_name = "TARGET|FILE|-")]
            target_or_file: String,

            /// Claims to retract. Use 'this=<DID>' to specify entity.
            /// 'field' retracts any value; 'field=value' retracts exact match only.
            #[arg(value_name = "FIELD[=VALUE]")]
            fields: Vec<String>,
        },

        /// Show current repository information
        #[command(alias = "st")]
        #[command(long_about = help::STATUS_LONG_ABOUT)]
        #[command(after_help = help::STATUS_AFTER_HELP)]
        Status,

        /// Manage spaces within a .carry/ repository
        #[command(alias = "s", hide = true)]
        #[command(long_about = help::SPACE_LONG_ABOUT)]
        #[command(after_help = help::SPACE_AFTER_HELP)]
        Space {
            #[command(subcommand)]
            command: SpaceCommands,
        },
    }

    #[derive(Subcommand)]
    pub enum SpaceCommands {
        /// List all spaces in the site
        #[command(alias = "l")]
        List,

        /// Create a new space
        #[command(alias = "c")]
        Create {
            /// Label for the new space
            #[arg(value_name = "LABEL")]
            label: Option<String>,
        },

        /// Switch active space
        #[command(alias = "s")]
        Switch {
            /// DID or label of the space to switch to
            #[arg(value_name = "DID|LABEL")]
            target: String,
        },

        /// Show current active space
        #[command(alias = "a")]
        Active,

        /// Delete a space (cannot delete the active space)
        #[command(alias = "d")]
        Delete {
            /// DID or label of the space to delete
            #[arg(value_name = "DID|LABEL")]
            target: String,

            /// Skip confirmation prompt
            #[arg(long, short)]
            yes: bool,
        },
    }
}

use inner::*;

#[cfg(target_arch = "wasm32")]
pub fn main() {}

#[cfg(not(target_arch = "wasm32"))]
use clap::{CommandFactory, Parser};
#[cfg(not(target_arch = "wasm32"))]
use clap_complete::env::CompleteEnv;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse();
    let repo_path = cli.repo.as_deref().map(std::path::Path::new);
    let space_flag = cli.space.as_deref();
    let format = &cli.format;

    match cli.command {
        Commands::Init { name } => {
            carry::init::execute(name, repo_path).await?;
        }
        Commands::Query { target, fields } => {
            let parsed_target = carry::target::Target::parse(&target)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::query_cmd::execute(&ctx, parsed_target, parsed.fields, format).await?;
        }
        Commands::Assert {
            target_or_file,
            fields,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::assert_cmd::execute(
                &ctx,
                first_arg,
                parsed.this_entity,
                parsed.entity_name,
                parsed.fields,
                format,
            )
            .await?;
        }
        Commands::Retract {
            target_or_file,
            fields,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::retract_cmd::execute(&ctx, first_arg, parsed.this_entity, parsed.fields, format)
                .await?;
        }
        Commands::Status => {
            carry::status_cmd::execute(repo_path, format).await?;
        }
        Commands::Space { command } => {
            let site = carry::space_cmd::resolve_site(repo_path)?;
            match command {
                SpaceCommands::List => {
                    carry::space_cmd::list(&site, format).await?;
                }
                SpaceCommands::Create { label } => {
                    carry::space_cmd::create(&site, label, format).await?;
                }
                SpaceCommands::Switch { target } => {
                    carry::space_cmd::switch(&site, &target).await?;
                }
                SpaceCommands::Active => {
                    carry::space_cmd::active(&site, format).await?;
                }
                SpaceCommands::Delete { target, yes } => {
                    carry::space_cmd::delete(&site, &target, yes).await?;
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — clap argument parsing
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::inner::*;
    use clap::Parser;

    // -- Query: --space not consumed by fields ------------------------------

    #[test]
    fn query_space_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "age",
            "--space",
            "test",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("test"));
        match cli.command {
            Commands::Query {
                ref target,
                ref fields,
            } => {
                assert_eq!(target, "com.app.person");
                assert_eq!(fields, &["name", "age"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn query_space_flag_before_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "--space",
            "test",
            "query",
            "com.app.person",
            "name",
            "age",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("test"));
        match cli.command {
            Commands::Query {
                ref target,
                ref fields,
            } => {
                assert_eq!(target, "com.app.person");
                assert_eq!(fields, &["name", "age"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn query_space_flag_with_did() {
        let did = "did:key:z6MkvSLQtPtAraTvgQwjz3ps9JBuY8a41STNikZ9bJdShNr6";
        let cli = Cli::try_parse_from(["carry", "query", "com.app.person", "name", "--space", did])
            .unwrap();
        assert_eq!(cli.space.as_deref(), Some(did));
        match cli.command {
            Commands::Query { ref fields, .. } => {
                assert_eq!(fields, &["name"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    // -- Query: --format not consumed by fields -----------------------------

    #[test]
    fn query_format_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.format, "json");
        match cli.command {
            Commands::Query { ref fields, .. } => {
                assert_eq!(fields, &["name"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    // -- Query: --space and --format together -------------------------------

    #[test]
    fn query_space_and_format_flags_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "age",
            "--space",
            "research",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("research"));
        assert_eq!(cli.format, "json");
        match cli.command {
            Commands::Query {
                ref target,
                ref fields,
            } => {
                assert_eq!(target, "com.app.person");
                assert_eq!(fields, &["name", "age"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    // -- Assert: --space not consumed by fields -----------------------------

    #[test]
    fn assert_space_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "assert",
            "com.app.person",
            "name=Alice",
            "age=28",
            "--space",
            "test",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("test"));
        match cli.command {
            Commands::Assert {
                ref target_or_file,
                ref fields,
            } => {
                assert_eq!(target_or_file, "com.app.person");
                assert_eq!(fields, &["name=Alice", "age=28"]);
            }
            _ => panic!("Expected Assert command"),
        }
    }

    #[test]
    fn assert_format_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "assert",
            "com.app.person",
            "name=Alice",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.format, "json");
        match cli.command {
            Commands::Assert { ref fields, .. } => {
                assert_eq!(fields, &["name=Alice"]);
            }
            _ => panic!("Expected Assert command"),
        }
    }

    // -- Retract: --space not consumed by fields ----------------------------

    #[test]
    fn retract_space_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "retract",
            "com.app.person",
            "this=did:key:zAlice",
            "age",
            "--space",
            "test",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("test"));
        match cli.command {
            Commands::Retract {
                ref target_or_file,
                ref fields,
            } => {
                assert_eq!(target_or_file, "com.app.person");
                assert_eq!(fields, &["this=did:key:zAlice", "age"]);
            }
            _ => panic!("Expected Retract command"),
        }
    }

    #[test]
    fn retract_format_flag_after_fields() {
        let cli = Cli::try_parse_from([
            "carry",
            "retract",
            "com.app.person",
            "this=did:key:zAlice",
            "name",
            "--format",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.format, "json");
        match cli.command {
            Commands::Retract { ref fields, .. } => {
                assert_eq!(fields, &["this=did:key:zAlice", "name"]);
            }
            _ => panic!("Expected Retract command"),
        }
    }

    // -- Fields with = values still parse correctly -------------------------

    #[test]
    fn query_filter_fields_with_space_flag() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name=Alice",
            "age",
            "--space",
            "my-space",
        ])
        .unwrap();
        assert_eq!(cli.space.as_deref(), Some("my-space"));
        match cli.command {
            Commands::Query { ref fields, .. } => {
                assert_eq!(fields, &["name=Alice", "age"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    // -- --format triples ----------------------------------------------------

    #[test]
    fn query_format_triples() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "--format",
            "triples",
        ])
        .unwrap();
        assert_eq!(cli.format, "triples");
        match cli.command {
            Commands::Query { ref fields, .. } => {
                assert_eq!(fields, &["name"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn query_format_triples_with_space() {
        let cli = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "age",
            "--format",
            "triples",
            "--space",
            "research",
        ])
        .unwrap();
        assert_eq!(cli.format, "triples");
        assert_eq!(cli.space.as_deref(), Some("research"));
        match cli.command {
            Commands::Query { ref fields, .. } => {
                assert_eq!(fields, &["name", "age"]);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn format_invalid_value_rejected() {
        let result = Cli::try_parse_from([
            "carry",
            "query",
            "com.app.person",
            "name",
            "--format",
            "csv",
        ]);
        assert!(result.is_err());
    }
}
