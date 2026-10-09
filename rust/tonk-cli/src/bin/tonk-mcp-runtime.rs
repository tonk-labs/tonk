//! Private JSON-lines child process for native MCP execution and account linking.
//! Account controls are host-only, never registered as model-callable MCP tools.
use std::{fs::OpenOptions, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use dialog_effects::storage::Directory;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tonk_cli::{
    account,
    mcp_runtime::Runtime,
    site::{SiteConfig, TonkSite},
    space::SpaceStore,
};

#[derive(Parser)]
#[command(group(clap::ArgGroup::new("mode").required(true).args(["development_data", "account_data"])))]
struct Args {
    /// Isolated development data, including a software identity. Not an account login.
    #[arg(long)]
    development_data: Option<PathBuf>,
    /// Host-selected tenant directory. Starts an identity without creating a space.
    #[arg(long)]
    account_data: Option<PathBuf>,
    /// Enable private account-space network controls. Requires host egress policy.
    #[arg(long, requires = "account_data")]
    account_spaces: bool,
}

enum Backend {
    Development(Box<Runtime>),
    Account {
        profile: Box<dialog_peer::Peer<dialog_storage::provider::storage::NativeSpace>>,
        config: Box<SiteConfig>,
        spaces_enabled: bool,
        selected: Option<Box<Runtime>>,
    },
}

impl Backend {
    async fn call(&mut self, request: Request) -> Result<Value> {
        match self {
            Self::Development(runtime) => runtime.call(&request.name, request.arguments).await,
            Self::Account {
                profile,
                config,
                spaces_enabled,
                selected,
            } => match request.name.as_str() {
                "account_status" if request.arguments == json!({}) => Ok(json!({
                    "deviceDid": profile.did().to_string(),
                    "rootDid": account::active_in(profile, &config.account_store).await?.map(|account| account.root_did),
                })),
                "account_authorize" => {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase", deny_unknown_fields)]
                    struct Authorization {
                        authorization: Value,
                        expected_account: Option<dialog_varsig::Did>,
                    }
                    let input: Authorization = serde_json::from_value(request.arguments)
                        .context("Invalid account authorization request")?;
                    Ok(serde_json::to_value(
                        account::accept_authorization_in(
                            profile,
                            &config.account_store,
                            &serde_json::to_vec(&input.authorization)?,
                            input.expected_account.as_ref(),
                        )
                        .await?,
                    )?)
                }
                "account_list_spaces" if *spaces_enabled && request.arguments == json!({}) => {
                    Ok(json!({"spaces": tonk_cli::account_spaces::list_with_config(config).await?}))
                }
                "account_open_space" if *spaces_enabled => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Selection {
                        subject: dialog_varsig::Did,
                    }
                    let selection: Selection = serde_json::from_value(request.arguments)
                        .context("Select a space by subject DID only")?;
                    anyhow::ensure!(
                        selected.is_none(),
                        "This process is already pinned to a space"
                    );
                    let runtime = Runtime::open_account_space(config, &selection.subject).await?;
                    *selected = Some(Box::new(runtime));
                    Ok(
                        json!({"subject": selection.subject.to_string(), "capabilities": Runtime::capabilities()}),
                    )
                }
                "account_push_space" | "account_pull_space"
                    if selected.is_some() && request.arguments == json!({}) =>
                {
                    let runtime = selected.as_mut().expect("selected runtime");
                    let outcome = if request.name == "account_push_space" {
                        runtime.push().await?
                    } else {
                        runtime.pull().await?
                    };
                    Ok(
                        json!({"before": outcome.before, "after": outcome.after, "advanced": outcome.advanced}),
                    )
                }
                name if (Runtime::capabilities().contains(&name) || name == "ui_read")
                    && selected.is_some() =>
                {
                    selected
                        .as_mut()
                        .expect("selected runtime")
                        .call(name, request.arguments)
                        .await
                }
                _ => anyhow::bail!(
                    "Account control unavailable. No space is authorized for tool execution."
                ),
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    name: String,
    arguments: Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let account_mode = args.account_data.is_some();
    let data_directory = args
        .account_data
        .or(args.development_data)
        .expect("required mode");
    let mut directory_builder = std::fs::DirBuilder::new();
    directory_builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory_builder.mode(0o700);
    }
    directory_builder.create(&data_directory)?;
    let root = data_directory.canonicalize()?;
    #[cfg(unix)]
    if account_mode {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            std::fs::metadata(&root)?.permissions().mode() & 0o077 == 0,
            "Hosted account directory must be private (mode 0700)"
        );
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("runtime.lock"))?;
    lock.try_lock()
        .context("This native runtime is already running")?;
    let mode = if account_mode {
        "account"
    } else {
        "development"
    };
    let mode_path = root.join("runtime-mode");
    match std::fs::read_to_string(&mode_path) {
        Ok(existing) => anyhow::ensure!(
            existing == mode,
            "Runtime directory belongs to another mode"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if account_mode {
                anyhow::ensure!(
                    std::fs::read_dir(&root)?
                        .all(|entry| entry.is_ok_and(|entry| entry.file_name() == "runtime.lock")),
                    "Hosted account requires an empty directory, not existing development state"
                );
            }
            std::fs::write(mode_path, mode)?;
        }
        Err(error) => return Err(error.into()),
    }
    let profile = root.join("profile");
    std::fs::create_dir_all(&profile)?;
    let config = SiteConfig {
        profile_name: if account_mode {
            "mcp-hosted"
        } else {
            "mcp-development"
        }
        .into(),
        profile_directory: Directory::At(profile.to_string_lossy().into_owned()),
        require_account: account_mode,
        provision_account_spaces: account_mode,
        account_store: SpaceStore::at(root.join("account")),
    };
    let (mut backend, greeting) = if account_mode {
        let profile = tonk_cli::site::open_profile(
            &config.profile_name,
            config.profile_directory.clone(),
            true,
        )
        .await?;
        account::resume_authorization_in(&profile, &config.account_store).await?;
        let greeting = json!({"deviceDid": profile.did().to_string(), "capabilities": []});
        (
            Backend::Account {
                profile: Box::new(profile),
                config: Box::new(config),
                spaces_enabled: args.account_spaces,
                selected: None,
            },
            greeting,
        )
    } else {
        let site_path = root.join("site");
        let site = if site_path.exists() {
            TonkSite::open_with(&site_path, config).await?
        } else {
            TonkSite::init_at_with(&site_path, config).await?
        };
        (
            Backend::Development(Box::new(Runtime::new(site))),
            json!({"capabilities": Runtime::capabilities()}),
        )
    };
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    output.write_all(format!("{greeting}\n").as_bytes()).await?;
    output.flush().await?;
    loop {
        let mut bytes = Vec::new();
        let count = (&mut input)
            .take(200_001)
            .read_until(b'\n', &mut bytes)
            .await?;
        if count == 0 {
            break;
        }
        if count > 200_000 || !bytes.ends_with(b"\n") {
            anyhow::bail!("Runtime request exceeds the limit");
        }
        let result = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => match backend.call(request).await {
                Ok(value) => json!({"result": value}),
                Err(error) => json!({"error": error.to_string()}),
            },
            Err(_) => json!({"error": "Invalid runtime request."}),
        };
        output.write_all(format!("{result}\n").as_bytes()).await?;
        output.flush().await?;
    }
    Ok(())
}
