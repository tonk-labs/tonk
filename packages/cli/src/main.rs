mod crypto;
mod delegation;
mod keystore;
mod login;
mod status;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { via, duration } => {
            login::execute(via, duration).await?;
        }
        Commands::Status { verbose } => {
            status::execute(verbose).await?;
        }
    }

    Ok(())
}
