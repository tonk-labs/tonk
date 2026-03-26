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

            /// Additional admin DIDs to delegate authority to at init time
            #[arg(long = "admin", value_name = "DID")]
            admins: Vec<String>,
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

            /// Output format for query results
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
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

            /// Output format for query results
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
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

            /// Output format for query results
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
        },

        /// Show current repository information
        #[command(alias = "st")]
        #[command(long_about = help::STATUS_LONG_ABOUT)]
        #[command(after_help = help::STATUS_AFTER_HELP)]
        Status {
            /// Output format for query results
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
        },

        /// Show or create your local identity
        #[command(alias = "id")]
        Identity {
            /// Discard cached identity and re-derive from passkey
            #[arg(long)]
            reset: bool,
        },

        /// Create an invite token for a collaborator
        Invite {
            /// DID of the user to invite (e.g., did:key:z6Mk...)
            #[arg(value_name = "INVITED_DID")]
            invited_did: String,
        },

        /// Join a space using an invite token
        Join {
            /// The invite token received from a collaborator
            #[arg(value_name = "TOKEN")]
            token: String,
        },

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
        List {
            /// Output format
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
        },

        /// Create a new space
        #[command(alias = "c")]
        Create {
            /// Label for the new space
            #[arg(value_name = "LABEL")]
            label: Option<String>,

            /// Output format
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
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
        Active {
            /// Output format
            #[arg(long, default_value = "yaml", value_parser = ["yaml", "json", "triples"])]
            format: String,
        },

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

    match cli.command {
        Commands::Init { name, admins } => {
            carry::init::execute(name, admins, repo_path).await?;
        }
        Commands::Query {
            target,
            fields,
            format,
        } => {
            let parsed_target = carry::target::Target::parse(&target)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::query_cmd::execute(&ctx, parsed_target, parsed.fields, &format).await?;
        }
        Commands::Assert {
            target_or_file,
            fields,
            format,
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
                &format,
            )
            .await?;
        }
        Commands::Retract {
            target_or_file,
            fields,
            format,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::retract_cmd::execute(
                &ctx,
                first_arg,
                parsed.this_entity,
                parsed.fields,
                &format,
            )
            .await?;
        }
        Commands::Status { format } => {
            carry::status_cmd::execute(repo_path, &format).await?;
        }
        Commands::Identity { reset } => {
            carry::identity_cmd::execute(reset).await?;
        }
        Commands::Invite { invited_did } => {
            let ctx = carry::site::SiteContext::resolve(repo_path, space_flag).await?;
            carry::invite_cmd::execute(&ctx, &invited_did).await?;
        }
        Commands::Join { token } => {
            carry::join_cmd::execute(&token, repo_path).await?;
        }
        Commands::Space { command } => {
            let site = carry::space_cmd::resolve_site(repo_path)?;
            match command {
                SpaceCommands::List { format } => {
                    carry::space_cmd::list(&site, &format).await?;
                }
                SpaceCommands::Create { label, format } => {
                    carry::space_cmd::create(&site, label, &format).await?;
                }
                SpaceCommands::Switch { target } => {
                    carry::space_cmd::switch(&site, &target).await?;
                }
                SpaceCommands::Active { format } => {
                    carry::space_cmd::active(&site, &format).await?;
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
                ..
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
                ..
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
        match cli.command {
            Commands::Query {
                ref fields,
                ref format,
                ..
            } => {
                assert_eq!(fields, &["name"]);
                assert_eq!(format, "json");
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
        match cli.command {
            Commands::Query {
                ref target,
                ref fields,
                ref format,
            } => {
                assert_eq!(target, "com.app.person");
                assert_eq!(fields, &["name", "age"]);
                assert_eq!(format, "json");
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
                ..
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
        match cli.command {
            Commands::Assert {
                ref fields,
                ref format,
                ..
            } => {
                assert_eq!(fields, &["name=Alice"]);
                assert_eq!(format, "json");
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
                ..
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
        match cli.command {
            Commands::Retract {
                ref fields,
                ref format,
                ..
            } => {
                assert_eq!(fields, &["this=did:key:zAlice", "name"]);
                assert_eq!(format, "json");
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
        match cli.command {
            Commands::Query {
                ref fields,
                ref format,
                ..
            } => {
                assert_eq!(fields, &["name"]);
                assert_eq!(format, "triples");
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
        assert_eq!(cli.space.as_deref(), Some("research"));
        match cli.command {
            Commands::Query {
                ref fields,
                ref format,
                ..
            } => {
                assert_eq!(fields, &["name", "age"]);
                assert_eq!(format, "triples");
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

    // -- Identity ---------------------------------------------------------------

    #[test]
    fn identity_parses() {
        let cli = Cli::try_parse_from(["carry", "identity"]).unwrap();
        match cli.command {
            Commands::Identity { reset } => {
                assert!(!reset);
            }
            _ => panic!("Expected Identity command"),
        }
    }

    #[test]
    fn identity_reset_flag() {
        let cli = Cli::try_parse_from(["carry", "identity", "--reset"]).unwrap();
        match cli.command {
            Commands::Identity { reset } => {
                assert!(reset);
            }
            _ => panic!("Expected Identity command"),
        }
    }

    #[test]
    fn identity_alias_id() {
        let cli = Cli::try_parse_from(["carry", "id"]).unwrap();
        assert!(matches!(cli.command, Commands::Identity { .. }));
    }

    // -- Invite -----------------------------------------------------------------

    #[test]
    fn invite_parses_did() {
        let did = "did:key:z6MkvSLQtPtAraTvgQwjz3ps9JBuY8a41STNikZ9bJdShNr6";
        let cli = Cli::try_parse_from(["carry", "invite", did]).unwrap();
        match cli.command {
            Commands::Invite { ref invited_did } => {
                assert_eq!(invited_did, did);
            }
            _ => panic!("Expected Invite command"),
        }
    }

    #[test]
    fn invite_with_repo_flag() {
        let cli = Cli::try_parse_from([
            "carry",
            "--repo",
            "/tmp/myrepo",
            "invite",
            "did:key:z6MkTest",
        ])
        .unwrap();
        assert_eq!(cli.repo.as_deref(), Some("/tmp/myrepo"));
        assert!(matches!(cli.command, Commands::Invite { .. }));
    }

    #[test]
    fn invite_missing_did_fails() {
        let result = Cli::try_parse_from(["carry", "invite"]);
        assert!(result.is_err());
    }

    // -- Join -------------------------------------------------------------------

    #[test]
    fn join_parses_token() {
        let tok = "carry_inv1_somebase64data";
        let cli = Cli::try_parse_from(["carry", "join", tok]).unwrap();
        match cli.command {
            Commands::Join { ref token } => {
                assert_eq!(token, tok);
            }
            _ => panic!("Expected Join command"),
        }
    }

    #[test]
    fn join_missing_token_fails() {
        let result = Cli::try_parse_from(["carry", "join"]);
        assert!(result.is_err());
    }
}
