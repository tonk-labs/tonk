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
            Some(SpaceCommands::Create { name, owners, description }) => {
                tonk_cli::space::create(name, owners, description).await?;
            }
            Some(SpaceCommands::Invite { email, space }) => {
                tonk_cli::space::invite(email, space).await?;
            }
            Some(SpaceCommands::Join { invite, profile }) => {
                tonk_cli::space::join(invite, profile).await?;
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
    }

    Ok(())
}
