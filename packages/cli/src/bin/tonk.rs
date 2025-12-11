use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tonk")]
#[command(about = "Tonk CLI - Authentication and management tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

    /// Operator commands
    Operator {
        #[command(subcommand)]
        command: OperatorCommands,
    },

    /// Inspect delegations and invites
    Inspect {
        #[command(subcommand)]
        command: InspectCommands,
    },

    /// Manage facts in the active space
    Fact {
        #[command(subcommand)]
        command: FactCommands,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Show the current session DID
    Current,

    /// Switch to a different session
    Set {
        /// Authority DID to switch to
        authority_did: String,
    },
}

#[derive(Subcommand)]
enum SpaceCommands {
    /// Show the current space DID
    Current,

    /// Switch to a different space
    Set {
        /// Space name or DID to switch to
        space: String,
    },

    /// Create a new space
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
}

#[derive(Subcommand)]
enum OperatorCommands {
    /// Generate a new operator key (base58btc encoded)
    Generate,
}

#[derive(Subcommand)]
enum InspectCommands {
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
}

#[derive(Subcommand)]
enum FactCommands {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { via } => {
            tonk_cli::login::execute(via).await?;
        }
        Commands::Session { command, verbose } => match command {
            None => {
                tonk_cli::session::list(verbose).await?;
            }
            Some(SessionCommands::Current) => {
                tonk_cli::session::show_current().await?;
            }
            Some(SessionCommands::Set { authority_did }) => {
                tonk_cli::session::set(authority_did).await?;
            }
        },
        Commands::Space { command } => match command {
            None => {
                tonk_cli::space::list().await?;
            }
            Some(SpaceCommands::Current) => {
                tonk_cli::space::show_current().await?;
            }
            Some(SpaceCommands::Set { space }) => {
                tonk_cli::space::set(space).await?;
            }
            Some(SpaceCommands::Create { name, owners, description }) => {
                tonk_cli::space::create(name, owners, description).await?;
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
        },
        Commands::Operator { command } => match command {
            OperatorCommands::Generate => {
                tonk_cli::operator::generate()?;
            }
        },
        Commands::Inspect { command } => match command {
            InspectCommands::Delegation { input } => {
                tonk_cli::delegation::inspect(input)?;
            }
            InspectCommands::Invite { path } => {
                tonk_cli::space::inspect_invite(path)?;
            }
        },
        Commands::Fact { command } => match command {
            FactCommands::Assert { the, of, is } => {
                let is_value = is.join(" ").trim().to_string();
                tonk_cli::fact::assert(the, of, is_value).await?;
            }
            FactCommands::Retract { the, of, is } => {
                let is_value = is.join(" ").trim().to_string();
                tonk_cli::fact::retract(the, of, is_value).await?;
            }
            FactCommands::Find { the, of, is, format } => {
                tonk_cli::fact::find(the, of, is, format).await?;
            }
        },
    }

    Ok(())
}
