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

        /// Session duration (e.g., "30d", "7d", "1h"). Defaults to 30 days.
        #[arg(long, default_value = "30d")]
        duration: String,
    },

    /// Show operator identity and current delegations
    Status {
        /// Show detailed information (file paths, full DIDs, etc.)
        #[arg(short, long)]
        verbose: bool,
    },

    /// Manage spaces (collaboration units)
    Space {
        #[command(subcommand)]
        command: SpaceCommands,
    },
}

#[derive(Subcommand)]
enum SpaceCommands {
    /// Create a new space
    Create {
        /// Name of the space
        name: String,

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { via, duration } => {
            tonk_cli::login::execute(via, duration).await?;
        }
        Commands::Status { verbose } => {
            tonk_cli::status::execute(verbose).await?;
        }
        Commands::Space { command } => match command {
            SpaceCommands::Create { name, description } => {
                tonk_cli::space::create(name, description).await?;
            }
            SpaceCommands::Invite { email, space } => {
                tonk_cli::space::invite(email, space).await?;
            }
        },
    }

    Ok(())
}
