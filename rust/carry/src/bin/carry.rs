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
        pub site: Option<String>,

        /// Output format for query results
        #[arg(long, global = true, default_value = "yaml", value_parser = ["yaml", "json"])]
        pub format: String,
    }

    #[derive(Subcommand)]
    pub enum Commands {
        /// Create a new .carry/ repository
        #[command(long_about = help::INIT_LONG_ABOUT)]
        #[command(after_help = help::INIT_AFTER_HELP)]
        Init {
            /// Label for the space (stored as xyz.tonk.carry/label claim)
            #[arg(value_name = "LABEL")]
            name: Option<String>,
        },

        /// Query entities by domain or concept
        #[command(long_about = help::QUERY_LONG_ABOUT)]
        #[command(after_help = help::QUERY_AFTER_HELP)]
        Query {
            /// Domain (contains '.') or concept name (no '.') to query
            #[arg(value_name = "TARGET")]
            target: String,

            /// Fields to output or filter. Use 'field' to include in output,
            /// 'field=value' to filter results.
            #[arg(
                trailing_var_arg = true,
                allow_hyphen_values = true,
                value_name = "FIELD[=VALUE]"
            )]
            fields: Vec<String>,
        },

        /// Assert claims on entities
        #[command(long_about = help::ASSERT_LONG_ABOUT)]
        #[command(after_help = help::ASSERT_AFTER_HELP)]
        Assert {
            /// Domain/concept, file path (.yaml/.yml/.json), or '-' for stdin
            #[arg(value_name = "TARGET|FILE|-")]
            target_or_file: String,

            /// Claims to assert. Use 'this=<DID>' to target existing entity,
            /// otherwise a new entity is created.
            #[arg(
                trailing_var_arg = true,
                allow_hyphen_values = true,
                value_name = "FIELD=VALUE"
            )]
            fields: Vec<String>,
        },

        /// Retract claims from entities
        #[command(long_about = help::RETRACT_LONG_ABOUT)]
        #[command(after_help = help::RETRACT_AFTER_HELP)]
        Retract {
            /// Domain/concept, file path (.yaml/.yml/.json), or '-' for stdin
            #[arg(value_name = "TARGET|FILE|-")]
            target_or_file: String,

            /// Claims to retract. Use 'this=<DID>' to specify entity.
            /// 'field' retracts any value; 'field=value' retracts exact match only.
            #[arg(
                trailing_var_arg = true,
                allow_hyphen_values = true,
                value_name = "FIELD[=VALUE]"
            )]
            fields: Vec<String>,
        },

        /// Show current site and space information
        #[command(long_about = help::STATUS_LONG_ABOUT)]
        #[command(after_help = help::STATUS_AFTER_HELP)]
        Status,
        // TODO: Add `Space` subcommand with nested commands:
        //   - `carry space list` - list all spaces in the site
        //   - `carry space create [LABEL]` - create a new space
        //   - `carry space switch <DID|LABEL>` - switch active space
        //   - `carry space active` - show current active space
        //   - `carry space delete <DID>` - delete a space (with confirmation)
        // Infrastructure already exists in site.rs: list_spaces(), create_space(),
        // set_active_space(), active_space_did(), etc.
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
            carry::init::execute(name, site_path).await?;
        }
        Commands::Query { target, fields } => {
            let parsed_target = carry::target::Target::parse(&target)?;
            let (parsed_fields, _this_entity) = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(site_path)?;
            carry::query_cmd::execute(&ctx, parsed_target, parsed_fields, format).await?;
        }
        Commands::Assert {
            target_or_file,
            fields,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let (parsed_fields, this_entity) = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(site_path)?;
            carry::assert_cmd::execute(&ctx, first_arg, this_entity, parsed_fields, format).await?;
        }
        Commands::Retract {
            target_or_file,
            fields,
        } => {
            let first_arg = carry::target::FirstArg::parse(&target_or_file)?;
            let (parsed_fields, this_entity) = carry::target::parse_fields(&fields)?;
            let ctx = carry::site::SiteContext::resolve(site_path)?;
            carry::retract_cmd::execute(&ctx, first_arg, this_entity, parsed_fields, format)
                .await?;
        }
        Commands::Status => {
            carry::status_cmd::execute(site_path, format).await?;
        }
    }

    Ok(())
}
