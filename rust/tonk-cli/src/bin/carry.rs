#[cfg(target_arch = "wasm32")]
mod inner {}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use clap::{Parser, Subcommand};

    #[derive(Parser)]
    #[command(name = "carry")]
    #[command(about = "Carry CLI", long_about = None)]
    pub struct Cli {
        #[command(subcommand)]
        pub command: Commands,

        /// Path to a specific repository (skips filesystem search)
        #[arg(long, global = true)]
        pub site: Option<String>,

        /// Output format: yaml (default) or json
        #[arg(long, global = true, default_value = "yaml")]
        pub format: String,
    }

    #[derive(Subcommand)]
    pub enum Commands {
        /// Create a new .carry/ repository in the current directory
        Init {
            /// Optional name for the space (asserted as a label)
            name: Option<String>,
        },

        /// Query entities by domain or concept
        Query {
            /// Target: domain (contains '.') or concept (no '.')
            target: String,

            /// Fields as name or name=value pairs
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            fields: Vec<String>,
        },

        /// Assert claims on entities
        Assert {
            /// Target, file path, or '-' for stdin
            target_or_file: String,

            /// Fields as this=<ENTITY> or name=value pairs
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            fields: Vec<String>,
        },

        /// Retract claims from entities
        Retract {
            /// Target, file path, or '-' for stdin
            target_or_file: String,

            /// Fields as this=<ENTITY> or name[=value] pairs
            #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
            fields: Vec<String>,
        },

        /// Show current site and space information
        Status,
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
    let site_path = cli.site.as_deref().map(std::path::Path::new);
    let format = &cli.format;

    match cli.command {
        Commands::Init { name } => {
            tonk_cli::init::execute(name, site_path).await?;
        }
        Commands::Query { target, fields } => {
            let parsed_target = tonk_cli::target::Target::parse(&target)?;
            let (parsed_fields, _this_entity) = tonk_cli::target::parse_fields(&fields)?;
            let ctx = tonk_cli::site::SiteContext::resolve(site_path)?;
            tonk_cli::query_cmd::execute(&ctx, parsed_target, parsed_fields, format).await?;
        }
        Commands::Assert {
            target_or_file,
            fields,
        } => {
            let first_arg = tonk_cli::target::FirstArg::parse(&target_or_file)?;
            let (parsed_fields, this_entity) = tonk_cli::target::parse_fields(&fields)?;
            let ctx = tonk_cli::site::SiteContext::resolve(site_path)?;
            tonk_cli::assert_cmd::execute(&ctx, first_arg, this_entity, parsed_fields, format)
                .await?;
        }
        Commands::Retract {
            target_or_file,
            fields,
        } => {
            let first_arg = tonk_cli::target::FirstArg::parse(&target_or_file)?;
            let (parsed_fields, this_entity) = tonk_cli::target::parse_fields(&fields)?;
            let ctx = tonk_cli::site::SiteContext::resolve(site_path)?;
            tonk_cli::retract_cmd::execute(&ctx, first_arg, this_entity, parsed_fields, format)
                .await?;
        }
        Commands::Status => {
            tonk_cli::status_cmd::execute(site_path, format).await?;
        }
    }

    Ok(())
}
