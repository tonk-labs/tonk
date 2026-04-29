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
            /// Reset and recreate the local identity
            #[arg(long)]
            reset: bool,
        },

        /// Create an invite URL for a collaborator
        Invite {
            /// DID of the collaborator to invite. If omitted, generates an
            /// open invite that anyone with the URL can claim.
            #[arg(value_name = "MEMBER")]
            member: Option<String>,

            /// Base URL for the invite link (default: https://tonk.xyz/join)
            #[arg(long, value_name = "URL")]
            url: Option<String>,
        },

        /// Join a repository using an invite URL
        Join {
            /// The invite URL received from a collaborator. If omitted,
            /// self-provisions an upstream for the space.
            #[arg(value_name = "INVITE-URL")]
            invite_url: Option<String>,
        },

        /// Manage sync remotes for this repository
        Remote {
            #[command(subcommand)]
            command: RemoteCommands,
        },

        /// Push local changes to the configured remote
        Push {},

        /// Pull and merge changes from the configured remote
        Pull {},
    }

    #[derive(Subcommand)]
    pub enum RemoteCommands {
        /// Register a sync destination for this repository
        Add {
            /// Name of the remote (e.g. "origin")
            #[arg(value_name = "NAME")]
            name: String,

            /// Remote URL. https:// for a UCAN-S3 access service
            /// (recommended), or s3:// for direct S3 (see --endpoint /
            /// --region / --bucket).
            #[arg(value_name = "URL")]
            url: String,

            /// Repository subject DID at the remote. Defaults to this
            /// repo's own DID (the common case); set this only when
            /// pointing at somebody else's repository.
            #[arg(long, value_name = "DID")]
            subject: Option<String>,

            /// S3 endpoint URL (only for s3:// remotes)
            #[arg(long, value_name = "URL")]
            endpoint: Option<String>,

            /// S3 region (only for s3:// remotes)
            #[arg(long, value_name = "REGION")]
            region: Option<String>,

            /// S3 bucket (only for s3:// remotes)
            #[arg(long, value_name = "BUCKET")]
            bucket: Option<String>,

            /// S3 access key ID (only for private s3:// remotes).
            /// WARNING: persisted in plaintext inside .carry/.
            #[arg(long = "access-key", value_name = "KEY")]
            access_key: Option<String>,

            /// S3 secret access key (only for private s3:// remotes).
            /// WARNING: persisted in plaintext inside .carry/.
            #[arg(long = "secret-key", value_name = "SECRET")]
            secret_key: Option<String>,

            /// Also wire this remote up as the sync target for push/pull
            /// (mirrors `git remote add -u`). Without this flag the
            /// remote is registered but no upstream is set.
            #[arg(long = "set-upstream", short = 'u')]
            set_upstream: bool,
        },

        /// List configured remotes
        #[command(alias = "ls")]
        List {},

        /// Show details of a specific remote
        Show {
            /// Name of the remote to inspect
            #[arg(value_name = "NAME")]
            name: String,
        },

        /// Set a remote as the sync target for push/pull
        #[command(name = "set-upstream")]
        SetUpstream {
            /// Name of the remote to use as upstream
            #[arg(value_name = "NAME")]
            name: String,
        },

        /// Remove a remote and clear its upstream link
        #[command(alias = "rm")]
        Remove {
            /// Name of the remote to remove
            #[arg(value_name = "NAME")]
            name: String,
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

    let command_name = match &cli.command {
        Commands::Init { .. } => "init",
        Commands::Query { .. } => "query",
        Commands::Assert { .. } => "assert",
        Commands::Retract { .. } => "retract",
        Commands::Status { .. } => "status",
        Commands::Identity { .. } => "identity",
        Commands::Invite { .. } => "invite",
        Commands::Join { .. } => "join",
        Commands::Remote { .. } => "remote",
        Commands::Push { .. } => "push",
        Commands::Pull { .. } => "pull",
    };

    // Best-effort telemetry: load existing identity for the blinded ID,
    // but fall back silently if no profile exists yet.
    let telemetry_handle = {
        use dialog_operator::Profile;
        use dialog_storage::provider::storage::{NativeSpace, Storage};
        let storage = Storage::<NativeSpace>::default();
        if let Ok(profile) = Profile::load("carry").perform(&storage).await {
            carry::telemetry::ping(profile.did().as_ref(), command_name)
        } else {
            None
        }
    };

    match cli.command {
        Commands::Init { name, admins } => {
            carry::init::execute(name, admins, repo_path, None, None).await?;
        }
        Commands::Query {
            target,
            fields,
            format,
        } => {
            let parsed_target = carry::target::Target::parse(&target)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::query_cmd::execute(&site, parsed_target, parsed.fields, &format).await?;
        }
        Commands::Assert {
            target_or_file,
            fields,
            format,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let parsed = carry::target::parse_fields(&fields)?;
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::assert_cmd::execute(
                &site,
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
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::retract_cmd::execute(
                &site,
                first_arg,
                parsed.this_entity,
                parsed.fields,
                &format,
            )
            .await?;
        }
        Commands::Status { format } => {
            carry::status_cmd::execute(repo_path, &format, None).await?;
        }
        Commands::Identity { reset } => {
            carry::identity_cmd::execute(reset).await?;
        }
        Commands::Invite { member, url } => {
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::invite_cmd::execute(&site, member.as_deref(), url.as_deref()).await?;
        }
        Commands::Join { invite_url } => {
            carry::join_cmd::execute(invite_url.as_deref(), repo_path, None).await?;
        }
        Commands::Remote { command } => match command {
            RemoteCommands::Add {
                name,
                url,
                subject,
                endpoint,
                region,
                bucket,
                access_key,
                secret_key,
                set_upstream,
            } => {
                let site = carry::site::Site::resolve(repo_path, None).await?;
                carry::remote_cmd::execute(
                    &site,
                    carry::remote_cmd::RemoteAddOptions {
                        name,
                        url,
                        subject,
                        s3_endpoint: endpoint,
                        s3_region: region,
                        s3_bucket: bucket,
                        s3_access_key: access_key,
                        s3_secret_key: secret_key,
                        set_upstream,
                    },
                )
                .await?;
            }
            RemoteCommands::List {} => {
                let site = carry::site::Site::resolve(repo_path, None).await?;
                carry::remote_cmd::execute_list(&site).await?;
            }
            RemoteCommands::Show { name } => {
                let site = carry::site::Site::resolve(repo_path, None).await?;
                carry::remote_cmd::execute_show(&site, &name).await?;
            }
            RemoteCommands::SetUpstream { name } => {
                let site = carry::site::Site::resolve(repo_path, None).await?;
                carry::remote_cmd::execute_set_upstream(&site, &name).await?;
            }
            RemoteCommands::Remove { name } => {
                let site = carry::site::Site::resolve(repo_path, None).await?;
                carry::remote_cmd::execute_remove(&site, &name).await?;
            }
        },
        Commands::Push {} => {
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::push_cmd::execute(&site).await?;
        }
        Commands::Pull {} => {
            let site = carry::site::Site::resolve(repo_path, None).await?;
            carry::pull_cmd::execute(&site).await?;
        }
    }

    // Wait for the telemetry ping to finish (up to its 500ms timeout)
    // so tokio doesn't cancel the spawned task on shutdown.
    if let Some(handle) = telemetry_handle {
        let _ = handle.await;
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

    // -- Assert: --format not consumed by fields -----------------------------

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

    // -- Retract: --format not consumed by fields ----------------------------

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
    fn invite_parses_with_member() {
        let cli = Cli::try_parse_from(["carry", "invite", "did:key:z6Mktest1234"]).unwrap();
        match cli.command {
            Commands::Invite {
                ref member,
                ref url,
            } => {
                assert_eq!(member.as_deref(), Some("did:key:z6Mktest1234"));
                assert!(url.is_none());
            }
            _ => panic!("Expected Invite command"),
        }
    }

    #[test]
    fn invite_parses_without_member() {
        let cli = Cli::try_parse_from(["carry", "invite"]).unwrap();
        match cli.command {
            Commands::Invite { ref member, .. } => {
                assert!(member.is_none());
            }
            _ => panic!("Expected Invite command"),
        }
    }

    #[test]
    fn invite_with_url_flag() {
        let cli =
            Cli::try_parse_from(["carry", "invite", "--url", "https://example.com/join"]).unwrap();
        match cli.command {
            Commands::Invite {
                ref member,
                ref url,
            } => {
                assert!(member.is_none());
                assert_eq!(url.as_deref(), Some("https://example.com/join"));
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
            "did:key:z6Mktest5678",
        ])
        .unwrap();
        assert_eq!(cli.repo.as_deref(), Some("/tmp/myrepo"));
        match cli.command {
            Commands::Invite { ref member, .. } => {
                assert_eq!(member.as_deref(), Some("did:key:z6Mktest5678"));
            }
            _ => panic!("Expected Invite command"),
        }
    }

    // -- Join -------------------------------------------------------------------

    #[test]
    fn join_parses_url() {
        let url = "https://tonk.xyz/join?access=abc123#secret";
        let cli = Cli::try_parse_from(["carry", "join", url]).unwrap();
        match cli.command {
            Commands::Join { ref invite_url } => {
                assert_eq!(invite_url.as_deref(), Some(url));
            }
            _ => panic!("Expected Join command"),
        }
    }

    #[test]
    fn join_without_url_succeeds() {
        let cli = Cli::try_parse_from(["carry", "join"]).unwrap();
        match cli.command {
            Commands::Join { ref invite_url } => {
                assert!(invite_url.is_none());
            }
            _ => panic!("Expected Join command"),
        }
    }
}
