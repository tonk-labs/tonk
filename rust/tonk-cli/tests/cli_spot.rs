//! CLI-level spot resolution: spawns the real `tonk` binary against
//! an isolated registry (`TONK_SPOTS_STATE`) and pins `TONK_SPOT` /
//! `--spot` explicitly, so these exercise the same precedence and
//! error text a human or an agent actually sees — not just the
//! `spot` module's in-process ops (covered by `tests/spot.rs`).

use std::path::Path;
use std::process::{Command, Output};

/// Path to the `tonk` binary under test. Mirrors `tests/telemetry.rs`:
/// the compile-time `CARGO_BIN_EXE_tonk` points into the sandbox the
/// tests were built in; a `cargo nextest archive` run on another
/// machine supplies the remapped location at runtime instead.
fn tonk_bin() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

/// A `tonk` invocation isolated from the developer's real state:
/// registry under `state_dir`, telemetry disabled and redirected,
/// update-check disabled and redirected — same hygiene as
/// `tests/telemetry.rs`'s `run_tonk_guide`. `HOME` is redirected too,
/// because the profile directory has no env override of its own and
/// resolves off the home dir; a test that actually opens a site would
/// otherwise write keys into the developer's real profile. `TONK_SPOT`
/// is always removed first so the outer environment never leaks in;
/// pass it via `extra_env` to pin it explicitly.
fn tonk_cmd(state_dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(tonk_bin());
    cmd.args(args)
        .env("TONK_SPOTS_STATE", state_dir)
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_TELEMETRY")
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("TONK_UPDATE_STATE", state_dir)
        .env("HOME", state_dir)
        .env_remove("TONK_SPOT");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd
}

fn run(state_dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
    tonk_cmd(state_dir, args, extra_env)
        .output()
        .expect("tonk binary runs")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Endpoints on the discard port: valid URLs that parse and register
/// fine, with nothing listening behind them. Anything that would reach
/// the wire fails fast instead of touching the network.
const DEAD_REMOTE: &str = "http://127.0.0.1:9/ucan/";
const OTHER_DEAD_REMOTE: &str = "http://127.0.0.1:9/other/";

/// Stand up a real spot through the CLI — `tonk spot new` writes the
/// site the rest of these invocations read — then register `remotes`
/// in order. `tonk remote add` wires the *first* remote as `main`'s
/// upstream and leaves it alone after that, so `remotes[0]` is the one
/// the repo pushes to.
fn spot_with_remotes(state_dir: &Path, remotes: &[(&str, &str)]) {
    let site = state_dir.join("site");
    let site = site.to_str().expect("utf-8 site path");
    let output = run(state_dir, &["spot", "new", "demo", "--site", site], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));

    for (name, endpoint) in remotes {
        let output = run(state_dir, &["remote", "add", name, endpoint], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
    }
}

/// Write a `spots.json` registry directly (bypassing the CLI) so
/// each test starts from a known, isolated fixture. `entries` are
/// `(name, absolute site path)` pairs; site paths need not contain a
/// real repo — resolution and its error text are what's under test,
/// not site opening.
fn write_registry(state_dir: &Path, entries: &[(&str, &Path)], current: Option<&str>) {
    let spots: String = entries
        .iter()
        .map(|(name, site)| {
            format!(
                "\"{name}\":{{\"site\":{site:?}}}",
                name = name,
                site = site.display().to_string(),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let current = match current {
        Some(name) => format!("\"current\":\"{name}\","),
        None => String::new(),
    };
    let json = format!("{{{current}\"spots\":{{{spots}}}}}");
    std::fs::write(state_dir.join("spots.json"), json).expect("write spots.json");
}

mod when_nothing_is_registered {
    use super::*;

    #[dialog_common::test]
    fn it_errors_with_spot_new_hint_when_nothing_registered() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("no spots registered"), "{stderr}");
        assert!(stderr.contains("--site"), "{stderr}");
    }
}

mod when_resolving_with_precedence {
    use super::*;

    fn two_spot_registry(state: &Path) {
        let a = state.join("site-a");
        let b = state.join("site-b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        write_registry(state, &[("a", &a), ("b", &b)], Some("b"));
    }

    #[dialog_common::test]
    fn it_prefers_the_flag_over_env_and_global() {
        let state = tempfile::tempdir().expect("tempdir");
        two_spot_registry(state.path());

        let output = run(
            state.path(),
            &["--spot", "a", "status"],
            &[("TONK_SPOT", "b")],
        );
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'a' (via flag"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_reads_tonk_spot_from_the_environment() {
        let state = tempfile::tempdir().expect("tempdir");
        two_spot_registry(state.path());

        let output = run(state.path(), &["status"], &[("TONK_SPOT", "a")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'a' (via env"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_treats_an_empty_tonk_spot_as_unset() {
        let state = tempfile::tempdir().expect("tempdir");
        two_spot_registry(state.path());

        let output = run(state.path(), &["status"], &[("TONK_SPOT", "")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("spot 'b' (via global"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_errors_on_an_unknown_spot_naming_available() {
        let state = tempfile::tempdir().expect("tempdir");
        let a = state.path().join("site-a");
        std::fs::create_dir_all(&a).expect("mkdir a");
        write_registry(state.path(), &[("a", &a)], None);

        let output = run(state.path(), &["status"], &[("TONK_SPOT", "nope")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("unknown spot 'nope'"), "{stderr}");
        assert!(stderr.contains("registered: a"), "{stderr}");
    }
}

mod when_joining {
    use super::*;

    #[dialog_common::test]
    fn it_rejects_a_duplicate_join_name_before_any_network_work() {
        let state = tempfile::tempdir().expect("tempdir");
        let a = state.path().join("site-a");
        std::fs::create_dir_all(&a).expect("mkdir a");
        write_registry(state.path(), &[("a", &a)], Some("a"));

        let output = run(
            state.path(),
            &["join", "not-a-real-url", "--name", "a"],
            &[],
        );
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("already exists"), "{stderr}");
    }
}

/// The mismatch warning `tonk invite` prints before it mints. These
/// run entirely offline: the mint's push to a discard-port upstream
/// fails right after, which is fine — the warning precedes it, so
/// nothing here asserts on the mint succeeding.
mod when_the_invite_remote_differs_from_the_upstream {
    use super::*;

    #[dialog_common::test]
    fn it_warns_the_recipient_may_miss_the_data() {
        let state = tempfile::tempdir().expect("tempdir");
        spot_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite", "--remote", "other"], &[]);
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("the invite embeds remote 'other' but the repo pushes to 'origin'"),
            "{stderr}"
        );
        assert!(
            stderr.contains("may join a deployment that has not received this data"),
            "{stderr}"
        );
    }
}

mod when_the_invite_remote_is_the_upstream {
    use super::*;

    #[dialog_common::test]
    fn it_mints_without_a_mismatch_warning() {
        let state = tempfile::tempdir().expect("tempdir");
        spot_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite", "--remote", "origin"], &[]);
        let stderr = stderr_of(&output);
        assert!(!stderr.contains("the invite embeds remote"), "{stderr}");
    }
}

mod when_inviting_without_a_remote {
    use super::*;

    #[dialog_common::test]
    fn it_mints_without_a_mismatch_warning() {
        let state = tempfile::tempdir().expect("tempdir");
        spot_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite", "--no-remote"], &[]);
        let stderr = stderr_of(&output);
        assert!(!stderr.contains("the invite embeds remote"), "{stderr}");
    }

    /// `--no-remote` is the way past the several-remotes error, so it
    /// can never be blocked by one. Nothing resolves an origin here, so
    /// the base falls back — out loud, not silently.
    #[dialog_common::test]
    fn it_warns_and_falls_back_when_several_remotes_are_registered() {
        let state = tempfile::tempdir().expect("tempdir");
        spot_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite", "--no-remote"], &[]);
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("warning: several remotes are registered"),
            "{stderr}"
        );
        assert!(stderr.contains("--base-url"), "{stderr}");
        assert!(
            !stderr.contains("error: several remotes are registered"),
            "{stderr}"
        );
    }
}

/// A bare `tonk invite` cannot pick between remotes, so it stops. The
/// error has to name both ways forward — `--remote` to choose one and
/// `--no-remote` to embed none.
mod when_several_remotes_are_registered {
    use super::*;

    #[dialog_common::test]
    fn it_names_both_ways_out_of_the_ambiguity() {
        let state = tempfile::tempdir().expect("tempdir");
        spot_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("--remote <NAME>"), "{stderr}");
        assert!(stderr.contains("--no-remote"), "{stderr}");
    }
}

/// End-to-end mints against the in-process access service the rest of
/// the suite uses (a local S3 plus UCAN server — no external network).
/// A live remote is the only way to see the minted URL at all: the
/// mint pushes to the upstream first, and shortening the link is a
/// request to the link's own origin.
#[cfg(feature = "integration-tests")]
mod when_minting_against_a_live_remote {
    use std::path::PathBuf;

    use anyhow::Result;
    use tonk_access_service::helpers::AccessServiceAddress;

    use super::*;

    /// Set a spot up and run one `tonk` invocation against it, all on
    /// the blocking pool. `#[dialog_common::test]` expands to
    /// `#[tokio::test]`, whose current-thread runtime is also hosting
    /// the access service — block that thread on a subprocess and the
    /// service the CLI is talking to can never answer.
    async fn mint_against(state_dir: PathBuf, endpoint: String, args: &[&str]) -> Output {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        tokio::task::spawn_blocking(move || {
            spot_with_remotes(&state_dir, &[("origin", &endpoint)]);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            run(&state_dir, &args, &[])
        })
        .await
        .expect("blocking tonk invocation joins")
    }

    /// The regression this whole path exists for: a bare `tonk invite`
    /// resolves the lone registered remote and builds the link on that
    /// remote's origin, not on the hardcoded default base.
    #[dialog_common::test]
    async fn it_builds_the_link_on_the_lone_remotes_origin(
        env: AccessServiceAddress,
    ) -> Result<()> {
        let state = tempfile::tempdir()?;
        let origin = env.access_service_url.trim_end_matches('/').to_owned();

        let output = mint_against(state.path().to_path_buf(), origin.clone(), &["invite"]).await;
        assert!(output.status.success(), "{}", stderr_of(&output));

        let stdout = stdout_of(&output);
        let url = stdout.trim();
        assert!(
            url.starts_with(&format!("{origin}/@/")),
            "short link sits on the remote's origin: {url}"
        );
        assert!(!url.contains("tonk.spot"), "not the default base: {url}");
        Ok(())
    }

    /// Resolve a `{origin}/@/{hash}` link back to what it stands for.
    /// The shortcut service answers with a relative `Location`, so the
    /// header carries the path and query the mint actually produced —
    /// the only way to inspect a link once it has been shortened.
    async fn shortcut_target(short_url: &str) -> Result<String> {
        let without_fragment = short_url.split('#').next().unwrap_or(short_url);
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get(without_fragment)
            .send()
            .await?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .expect("shortcut answers with a Location")
            .to_str()?
            .to_owned();
        Ok(location)
    }

    /// `--no-remote` drops the embedded endpoint and nothing else. The
    /// link stays on the remote's origin — rerouting it to the
    /// canonical base would strand the recipient on a deployment that
    /// has never seen this repo, the split this path exists to avoid.
    #[dialog_common::test]
    async fn it_keeps_the_lone_remotes_origin_and_embeds_no_remote(
        env: AccessServiceAddress,
    ) -> Result<()> {
        let state = tempfile::tempdir()?;
        let origin = env.access_service_url.trim_end_matches('/').to_owned();

        let output = mint_against(
            state.path().to_path_buf(),
            origin.clone(),
            &["invite", "--no-remote"],
        )
        .await;
        assert!(output.status.success(), "{}", stderr_of(&output));

        let stdout = stdout_of(&output);
        let url = stdout.trim();
        assert!(
            url.starts_with(&format!("{origin}/@/")),
            "short link sits on the remote's origin: {url}"
        );
        assert!(!url.contains("tonk.spot"), "not the default base: {url}");

        let target = shortcut_target(url).await?;
        assert!(target.contains("access="), "still a real invite: {target}");
        assert!(!target.contains("remote="), "no remote embedded: {target}");
        Ok(())
    }

    /// `--base-url` still wins over the remote-derived origin.
    #[dialog_common::test]
    async fn it_prefers_an_explicit_base_url_over_the_remote(
        env: AccessServiceAddress,
    ) -> Result<()> {
        let state = tempfile::tempdir()?;
        let origin = env.access_service_url.trim_end_matches('/').to_owned();

        let output = mint_against(
            state.path().to_path_buf(),
            origin,
            &["invite", "--base-url", "http://127.0.0.1:9/join"],
        )
        .await;
        assert!(output.status.success(), "{}", stderr_of(&output));

        let stdout = stdout_of(&output);
        let url = stdout.trim();
        assert!(
            url.starts_with("http://127.0.0.1:9/join"),
            "explicit base wins: {url}"
        );
        // Nothing serves the discard port, so the link stays long.
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("could not shorten the invite URL"),
            "{stderr}"
        );
        Ok(())
    }
}
