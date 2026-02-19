#[cfg(target_arch = "wasm32")]
mod inner {}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use clap::{Parser, Subcommand};

    #[derive(Parser)]
    #[command(name = "tonk")]
    #[command(about = "Tonk CLI - Authentication and management tool", long_about = None)]
    pub struct Cli {
        #[command(subcommand)]
        pub command: Commands,

        /// Output results as JSON (for machine/agent consumption)
        #[arg(long, global = true)]
        pub json: bool,
    }

    #[derive(Subcommand)]
    pub enum Commands {
        /// Authenticate and obtain delegated capabilities
        Login {
            /// Optional authentication URL (e.g., "https://auth.tonk.xyz").
            /// If not provided, serves auth page locally.
            #[arg(long)]
            via: Option<String>,
        },

        /// Manage sessions (authority contexts)
        Session {
            #[command(subcommand)]
            command: Option<SessionCommands>,

            /// Show verbose output including delegation chains
            #[arg(short, long)]
            verbose: bool,
        },

        /// Manage spaces (collaboration units)
        Space {
            #[command(subcommand)]
            command: Option<SpaceCommands>,
        },

        /// Manage concept definitions (schemas for structured data)
        Concept {
            #[command(subcommand)]
            command: Option<ConceptCommands>,
        },

        /// Inspect attribute definitions
        Attribute {
            #[command(subcommand)]
            command: Option<AttributeCommands>,
        },

        /// Manage deductive rules between concepts
        Rule {
            #[command(subcommand)]
            command: Option<RuleCommands>,
        },

        /// Import concepts from a YAML file
        Import {
            /// Path to a YAML file containing concept definitions
            file: String,

            /// Overwrite existing concepts instead of failing
            #[arg(long)]
            force: bool,
        },

        /// Create a new entity
        Create {
            /// Concept name (e.g., "Task")
            concept: String,

            /// Field values as key=value pairs (e.g., title="Fix bug" status=todo)
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            fields: Vec<String>,

            /// Read field values from a JSON file
            #[arg(long, short)]
            file: Option<String>,

            /// Read field values from stdin as JSON
            #[arg(long)]
            stdin: bool,
        },

        /// Query entities with optional selectors
        Query {
            /// Concept name (e.g., "Task")
            concept: String,

            /// Selectors as key=value pairs (e.g., status=todo priority=high)
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            selectors: Vec<String>,
        },

        /// Show details of an entity by ID
        Show {
            /// Entity ID (did:key:...)
            id: String,
        },

        /// Assert new attributes on an entity
        Assert {
            /// Entity ID (did:key:...)
            id: String,

            /// Field values to update as key=value pairs
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            fields: Vec<String>,
        },

        /// Retract all known attributes of the entity
        Retract {
            /// Entity ID (did:key:...)
            id: String,
        },

        /// Batch operations on entities (create, update, delete multiple at once)
        Batch {
            #[command(subcommand)]
            command: BatchCommands,
        },

        /// Manage remotes for syncing spaces
        Remote {
            #[command(subcommand)]
            command: RemoteCommands,
        },

        /// Pull changes from the upstream remote
        Pull,

        /// Push local changes to the upstream remote
        Push,

        /// Sync with upstream (pull then push)
        Sync,

        /// Show current context (operator, session, space, remote)
        Status,

        /// Developer tools (raw fact operations, inspection, operator keys)
        Dev {
            #[command(subcommand)]
            command: DevCommands,
        },
    }

    #[derive(Subcommand)]
    pub enum SessionCommands {
        /// Show the current session DID
        Current,

        /// Switch to a different session
        Set {
            /// Authority DID to switch to
            authority_did: String,
        },
    }

    #[derive(Subcommand)]
    pub enum SpaceCommands {
        /// Show the current space DID
        Current,

        /// Load an existing space by name or DID (fails if not found)
        Load {
            /// Space name or DID to load
            space: String,
        },

        /// Create a new space (fails if a space with the same name already exists)
        Create {
            /// Name of the space
            name: String,

            /// Owner DIDs (did:key identifiers). If not provided, will prompt interactively.
            /// The active authority is always included as an owner.
            #[arg(short, long)]
            owners: Option<Vec<String>>,

            /// Optional description
            #[arg(short, long)]
            description: Option<String>,
        },

        /// Open a space: load if it exists, otherwise create it
        Open {
            /// Name of the space
            name: String,

            /// Owner DIDs (did:key identifiers). If not provided, will prompt interactively.
            /// The active authority is always included as an owner.
            /// Only used when creating a new space.
            #[arg(short, long)]
            owners: Option<Vec<String>>,

            /// Optional description (only used when creating a new space)
            #[arg(short, long)]
            description: Option<String>,
        },

        /// Invite a collaborator to a space
        Invite {
            /// Email address of the invitee (e.g., alice@example.com)
            email: String,

            /// Name of the space (defaults to active space)
            #[arg(short, long)]
            space: Option<String>,
        },

        /// Join a space using an invitation file
        Join {
            /// Path to the invitation file
            #[arg(short, long)]
            invite: String,

            /// Profile to join with (defaults to active profile)
            #[arg(short, long)]
            profile: Option<String>,
        },

        /// Delete a space
        Delete {
            /// Space name or DID to delete
            space: String,

            /// Skip confirmation prompt
            #[arg(short, long)]
            force: bool,
        },

        /// Delegate access to a space for another operator
        Delegate {
            /// DID of the operator to delegate to (did:key:...)
            #[arg(long)]
            to: String,

            /// Space name or DID (defaults to active space)
            #[arg(long)]
            space: Option<String>,

            /// Grant read-only access (default is read-write)
            #[arg(long)]
            read_only: bool,

            /// Output file path (defaults to stdout as base64)
            #[arg(short, long)]
            output: Option<String>,
        },
    }

    #[derive(Subcommand)]
    pub enum ConceptCommands {
        /// Define a new concept (schema)
        Define {
            /// Concept name (e.g., "Task", "Contact")
            name: String,

            /// Attribute names (auto-prefixed with concept namespace)
            #[arg(trailing_var_arg = true)]
            attributes: Vec<String>,

            /// Description of the concept (required for discoverability)
            #[arg(short, long)]
            description: String,
        },

        /// Show details of a concept
        Show {
            /// Concept name
            name: String,
        },

        /// Add attributes to an existing concept
        Extend {
            /// Concept name
            name: String,

            /// New attribute names to add
            #[arg(trailing_var_arg = true, required = true)]
            attributes: Vec<String>,
        },

        /// Delete a concept
        Delete {
            /// Concept name
            name: String,

            /// Also delete all entities of this concept
            #[arg(short, long)]
            force: bool,
        },
    }

    #[derive(Subcommand)]
    pub enum AttributeCommands {
        /// Show details of a specific attribute
        Show {
            /// Attribute name (qualified like "recipe/title", or short like "title" with --concept)
            name: String,

            /// Concept name (required when using short attribute names)
            #[arg(long, short)]
            concept: Option<String>,
        },
    }

    #[derive(Subcommand)]
    pub enum RuleCommands {
        /// Define a new deductive rule
        Define {
            /// Rule name (e.g., "safe-meals", "high-priority-tasks")
            name: String,

            /// Read rule definition from a JSON file
            #[arg(long, short)]
            file: Option<String>,

            /// Read rule definition from stdin as JSON
            #[arg(long)]
            stdin: bool,

            /// Description of the rule (required for discoverability)
            #[arg(short, long)]
            description: String,
        },

        /// Show details of a rule
        Show {
            /// Rule name
            name: String,
        },

        /// Delete a rule
        Delete {
            /// Rule name
            name: String,
        },
    }

    #[derive(Subcommand)]
    pub enum DevCommands {
        /// Raw fact operations on the active space
        Fact {
            #[command(subcommand)]
            command: FactCommands,
        },

        /// Operator key management
        Operator {
            #[command(subcommand)]
            command: OperatorCommands,
        },

        /// Inspect delegations, invites, and CBOR data
        Inspect {
            #[command(subcommand)]
            command: InspectCommands,
        },
    }

    #[derive(Subcommand)]
    pub enum OperatorCommands {
        /// Generate a new operator key (base58btc encoded)
        Generate,
    }

    #[derive(Subcommand)]
    pub enum InspectCommands {
        /// Inspect a delegation (base64-encoded CBOR or .cbor file)
        Delegation {
            /// Base64-encoded CBOR delegation string or path to .cbor file
            input: String,
        },

        /// Inspect an invite file
        Invite {
            /// Path to .invite file
            path: String,
        },

        /// Inspect CBOR data and display as JSON
        Cbor {
            /// Path to .cbor file or base64-encoded CBOR string
            input: String,
        },
    }

    #[derive(Subcommand)]
    pub enum FactCommands {
        /// Assert a fact into the active space
        Assert {
            /// The attribute (e.g., "user/name")
            #[arg(long, allow_hyphen_values = true)]
            the: String,

            /// The entity identifier. Can be:
            /// - ~/path - derives entity from operator signature
            /// - URI (e.g., did:key:..., https://...) - used as-is
            /// - any string - hashed to create entity
            #[arg(long, allow_hyphen_values = true)]
            of: String,

            /// The value to assert (all remaining words joined with spaces)
            #[arg(long, required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
            is: Vec<String>,
        },

        /// Retract a fact from the active space
        Retract {
            /// The attribute (e.g., "user/name")
            #[arg(long, allow_hyphen_values = true)]
            the: String,

            /// The entity identifier
            #[arg(long, allow_hyphen_values = true)]
            of: String,

            /// The value to retract (all remaining words joined with spaces)
            #[arg(long, required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
            is: Vec<String>,
        },

        /// Batch assert/retract facts from a YAML file or stdin (JSON lines)
        Batch {
            /// Path to a YAML file of {the, of, is} triples (reads JSON Lines from stdin if omitted)
            #[arg(short = 'f', long)]
            file: Option<String>,
        },

        /// Find facts in the active space
        Find {
            /// Filter by attribute (e.g., "user/name")
            #[arg(long, allow_hyphen_values = true)]
            the: Option<String>,

            /// Filter by entity identifier
            #[arg(long, allow_hyphen_values = true)]
            of: Option<String>,

            /// Filter by value
            #[arg(long, allow_hyphen_values = true)]
            is: Option<String>,

            /// Format for decoding byte values (cbor, json, text, ucan)
            #[arg(long, short)]
            format: Option<String>,
        },
    }

    #[derive(Subcommand)]
    pub enum BatchCommands {
        /// Create multiple entities of a concept from a JSON array
        Create {
            /// Concept name (e.g., "Task")
            concept: String,

            /// Read entity data from a JSON file (array of objects)
            #[arg(long, short)]
            file: Option<String>,

            /// Read entity data from stdin as JSON array
            #[arg(long)]
            stdin: bool,
        },

        /// Update multiple entities from a JSON array (each object must include "id")
        Update {
            /// Concept name (e.g., "Task")
            concept: String,

            /// Read update data from a JSON file (array of objects with "id" field)
            #[arg(long, short)]
            file: Option<String>,

            /// Read update data from stdin as JSON array
            #[arg(long)]
            stdin: bool,
        },

        /// Delete multiple entities from a JSON array of IDs
        Delete {
            /// Concept name (e.g., "Task")
            concept: String,

            /// Read entity IDs from a JSON file (array of ID strings)
            #[arg(long, short)]
            file: Option<String>,

            /// Read entity IDs from stdin as JSON array
            #[arg(long)]
            stdin: bool,
        },
    }

    #[derive(Subcommand)]
    pub enum RemoteCommands {
        /// Add a remote for syncing the active space
        Add {
            /// Name for the remote (e.g., "origin")
            name: String,

            /// UCAN access service URL (e.g., https://access.tonk.xyz).
            /// When provided, uses UCAN delegation instead of raw S3 credentials.
            #[arg(long, conflicts_with_all = ["endpoint", "bucket", "region", "access_key_id", "secret_access_key"])]
            service_url: Option<String>,

            /// S3 endpoint URL (e.g., https://s3.amazonaws.com)
            #[arg(long, visible_alias = "host")]
            endpoint: Option<String>,

            /// S3 bucket name
            #[arg(long)]
            bucket: Option<String>,

            /// AWS region (default: us-east-1)
            #[arg(long)]
            region: Option<String>,

            /// AWS Access Key ID
            #[arg(long)]
            access_key_id: Option<String>,

            /// AWS Secret Access Key
            #[arg(long)]
            secret_access_key: Option<String>,
        },

        /// Show the current upstream remote configuration
        Show,

        /// Remove the upstream remote
        Delete,

        /// Edit the upstream remote configuration
        Edit,
    }
}

use inner::*;

#[cfg(target_arch = "wasm32")]
pub fn main() {}

#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let json = cli.json;

    match cli.command {
        Commands::Login { via } => {
            tonk_cli::login::execute(via).await?;
        }
        Commands::Session { command, verbose } => match command {
            None => {
                tonk_cli::session::list(verbose, json).await?;
            }
            Some(SessionCommands::Current) => {
                tonk_cli::session::show_current(json).await?;
            }
            Some(SessionCommands::Set { authority_did }) => {
                tonk_cli::session::set(authority_did).await?;
            }
        },
        Commands::Space { command } => match command {
            None => {
                tonk_cli::space::list(json).await?;
            }
            Some(SpaceCommands::Current) => {
                tonk_cli::space::show_current(json).await?;
            }
            Some(SpaceCommands::Load { space }) => {
                tonk_cli::space::load(space).await?;
            }
            Some(SpaceCommands::Create {
                name,
                owners,
                description,
            }) => {
                tonk_cli::space::create(name, owners, description, json).await?;
            }
            Some(SpaceCommands::Open {
                name,
                owners,
                description,
            }) => {
                tonk_cli::space::open(name, owners, description, json).await?;
            }
            Some(SpaceCommands::Invite { email, space }) => {
                tonk_cli::space::invite(email, space).await?;
            }
            Some(SpaceCommands::Join { invite, profile }) => {
                tonk_cli::space::join(invite, profile).await?;
            }
            Some(SpaceCommands::Delete { space, force }) => {
                tonk_cli::space::delete(space, force).await?;
            }
            Some(SpaceCommands::Delegate {
                to,
                space,
                read_only,
                output,
            }) => {
                tonk_cli::space::delegate(to, space, read_only, output).await?;
            }
        },
        Commands::Concept { command } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            match command {
                None => {
                    // `tonk concept` with no subcommand lists all concepts
                    tonk_cli::concept::list(&ctx, json).await?;
                }
                Some(ConceptCommands::Define {
                    name,
                    attributes,
                    description,
                }) => {
                    tonk_cli::concept::define(&ctx, name, attributes, description, json).await?;
                }
                Some(ConceptCommands::Show { name }) => {
                    tonk_cli::concept::show(&ctx, name, json).await?;
                }
                Some(ConceptCommands::Extend { name, attributes }) => {
                    tonk_cli::concept::extend(&ctx, name, attributes, json).await?;
                }
                Some(ConceptCommands::Delete { name, force }) => {
                    tonk_cli::concept::delete(&ctx, name, force, json).await?;
                }
            }
        }
        Commands::Attribute { command } => match command {
            None => {
                tonk_cli::attribute::list(json).await?;
            }
            Some(AttributeCommands::Show { name, concept }) => {
                tonk_cli::attribute::show(name, concept, json).await?;
            }
        },
        Commands::Rule { command } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            match command {
                None => {
                    // `tonk rule` with no subcommand lists all rules
                    tonk_cli::rule::list(&ctx, json).await?;
                }
                Some(RuleCommands::Define {
                    name,
                    file,
                    stdin,
                    description,
                }) => {
                    tonk_cli::rule::define(&ctx, name, file, stdin, description, json).await?;
                }
                Some(RuleCommands::Show { name }) => {
                    tonk_cli::rule::show(&ctx, name, json).await?;
                }
                Some(RuleCommands::Delete { name }) => {
                    tonk_cli::rule::delete(&ctx, name, json).await?;
                }
            }
        }
        Commands::Import { file, force } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::import::import(&ctx, file, force, json).await?;
        }
        Commands::Create {
            concept,
            fields,
            file,
            stdin,
        } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::entity::create(&ctx, concept, fields, file, stdin, json).await?;
        }
        Commands::Query { concept, selectors } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::entity::query(&ctx, concept, selectors, json).await?;
        }
        Commands::Show { id } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::entity::show(&ctx, id, json).await?;
        }
        Commands::Assert { id, fields } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::entity::assert(&ctx, id, fields, json).await?;
        }
        Commands::Retract { id } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            tonk_cli::entity::retract(&ctx, id, json).await?;
        }
        Commands::Batch { command } => {
            let ctx = tonk_cli::schema::get_space_context()?;
            match command {
                BatchCommands::Create {
                    concept,
                    file,
                    stdin,
                } => {
                    tonk_cli::batch::batch_create(&ctx, concept, file, stdin, json).await?;
                }
                BatchCommands::Update {
                    concept,
                    file,
                    stdin,
                } => {
                    tonk_cli::batch::batch_update(&ctx, concept, file, stdin, json).await?;
                }
                BatchCommands::Delete {
                    concept,
                    file,
                    stdin,
                } => {
                    tonk_cli::batch::batch_delete(&ctx, concept, file, stdin, json).await?;
                }
            }
        }
        Commands::Dev { command } => match command {
            DevCommands::Fact { command } => {
                let ctx = tonk_cli::schema::get_space_context()?;
                match command {
                    FactCommands::Assert { the, of, is } => {
                        let is_value = is.join(" ").trim().to_string();
                        tonk_cli::fact::assert(&ctx, the, of, is_value, json).await?;
                    }
                    FactCommands::Retract { the, of, is } => {
                        let is_value = is.join(" ").trim().to_string();
                        tonk_cli::fact::retract(&ctx, the, of, is_value, json).await?;
                    }
                    FactCommands::Find {
                        the,
                        of,
                        is,
                        format,
                    } => {
                        tonk_cli::fact::find(&ctx, the, of, is, format, json).await?;
                    }
                    FactCommands::Batch { file } => {
                        tonk_cli::fact::batch(&ctx, file, json).await?;
                    }
                }
            }
            DevCommands::Operator { command } => match command {
                OperatorCommands::Generate => {
                    tonk_cli::operator::generate()?;
                }
            },
            DevCommands::Inspect { command } => match command {
                InspectCommands::Delegation { input } => {
                    tonk_cli::delegation::inspect(input)?;
                }
                InspectCommands::Invite { path } => {
                    tonk_cli::space::inspect_invite(path)?;
                }
                InspectCommands::Cbor { input } => {
                    tonk_cli::inspect::cbor(input)?;
                }
            },
        },
        Commands::Remote { command } => match command {
            RemoteCommands::Add {
                name,
                service_url,
                endpoint,
                bucket,
                region,
                access_key_id,
                secret_access_key,
            } => {
                tonk_cli::remote::add(
                    name,
                    service_url,
                    endpoint,
                    bucket,
                    region,
                    access_key_id,
                    secret_access_key,
                )
                .await?;
            }
            RemoteCommands::Show => {
                tonk_cli::remote::show().await?;
            }
            RemoteCommands::Delete => {
                tonk_cli::remote::delete().await?;
            }
            RemoteCommands::Edit => {
                tonk_cli::remote::edit().await?;
            }
        },
        Commands::Pull => {
            tonk_cli::remote::pull().await?;
        }
        Commands::Push => {
            tonk_cli::remote::push().await?;
        }
        Commands::Sync => {
            tonk_cli::remote::sync().await?;
        }
        Commands::Status => {
            tonk_cli::status::execute(json).await?;
        }
    }

    Ok(())
}
