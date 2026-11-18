//! `tonk_install_extension`: hand the Tonk Desktop extension to Claude Desktop.
//!
//! Claude Desktop renders MCP Apps views only from servers it runs as
//! extensions, never from a plugin's server, so live views in Desktop chat
//! need the extension installed beside the plugin. The launcher installs the
//! extension (`tonk-peer.mcpb`, built with the same release) beside this
//! binary, verified with the rest of the release archive. This tool opens it
//! with Claude Desktop, which shows its own install screen: the user confirms
//! there. The agent calls it when the user asks to see something and live views
//! need the extension; the install screen is the user's confirmation.
use std::path::{Path, PathBuf};

use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::{Value, json};

pub const NAME: &str = "tonk_install_extension";

/// The extension's Desktop identity: `local.mcpb.<author>.<name>`, slugged
/// from the bundle manifest (author "Tonk Labs", name "tonk-agent-peer", the
/// name the extension has had since its first build, so updates replace it).
const EXTENSION_ID: &str = "local.mcpb.tonk-labs.tonk-agent-peer";
const BUNDLE: &str = "tonk-peer.mcpb";
const DOWNLOAD: &str =
    "https://github.com/tonk-labs/tonk/releases/download/tonk-agent-peer-dev/tonk-peer.mcpb";

pub fn tool() -> Tool {
    serde_json::from_value(json!({
        "name": NAME,
        "title": "Install the Tonk extension in Claude Desktop",
        "description": "Opens Claude Desktop's install screen for the Tonk extension, which live views (tonk_display, tonk_show) need in Desktop chat. Call it when the user asks to display or show something and only the plugin's copy of the tools is present, at most once per conversation. Nothing is installed until the user confirms in Desktop's own install screen.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        "annotations": {
            "title": "Install the Tonk extension in Claude Desktop",
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        },
    }))
    .expect("extension tool shape")
}

pub fn call() -> CallToolResult {
    match install() {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
    }
}

fn install() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err(format!(
            "Installing the extension from here works on macOS only. On this computer, download {DOWNLOAD} and open it with Claude Desktop."
        ));
    }
    if let Some(state) = installed() {
        return Ok(match state {
            Installed::Enabled => "The Tonk extension is already installed. Live views need a new chat: start one and ask again.".to_owned(),
            Installed::Disabled => "The Tonk extension is installed but turned off. Turn it on in Claude Desktop under Settings → Extensions, then start a new chat.".to_owned(),
        });
    }
    let bundle = bundle().ok_or_else(|| {
        format!("This peer has no copy of the extension to install. Download {DOWNLOAD} and open it with Claude Desktop.")
    })?;
    open(&bundle)?;
    Ok("Claude Desktop is showing the Tonk extension's install screen. Choose Install there, then start a new chat for live views. Nothing is installed until you confirm.".to_owned())
}

enum Installed {
    Enabled,
    Disabled,
}

/// Desktop keeps each extension in `Claude Extensions/<id>` and its settings,
/// including whether it is turned on, in `Claude Extensions Settings/<id>.json`.
fn installed() -> Option<Installed> {
    let claude = desktop_data()?;
    if !claude.join("Claude Extensions").join(EXTENSION_ID).is_dir() {
        return None;
    }
    let settings = claude
        .join("Claude Extensions Settings")
        .join(format!("{EXTENSION_ID}.json"));
    let enabled = std::fs::read(settings)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| settings.get("isEnabled").and_then(Value::as_bool))
        .unwrap_or(true);
    Some(if enabled {
        Installed::Enabled
    } else {
        Installed::Disabled
    })
}

fn desktop_data() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join("Library")
            .join("Application Support")
            .join("Claude"),
    )
}

/// The bundle the launcher installed beside this binary, or one named by
/// `TONK_AGENT_PEER_EXTENSION` (development).
fn bundle() -> Option<PathBuf> {
    std::env::var_os("TONK_AGENT_PEER_EXTENSION")
        .map(PathBuf::from)
        .or_else(|| Some(std::env::current_exe().ok()?.parent()?.join(BUNDLE)))
        .filter(|path| path.is_file())
}

/// Open the bundle with Claude Desktop, which answers with its install screen.
fn open(bundle: &Path) -> Result<(), String> {
    let opened = |args: &[&std::ffi::OsStr]| {
        std::process::Command::new("open")
            .args(args)
            .status()
            .is_ok_and(|status| status.success())
    };
    if opened(&["-a".as_ref(), "Claude".as_ref(), bundle.as_os_str()]) {
        return Ok(());
    }
    Err(format!(
        "Could not open Claude Desktop with {}. Open that file yourself, or download {DOWNLOAD} and open it.",
        bundle.display()
    ))
}
