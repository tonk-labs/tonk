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
/// `tests/telemetry.rs`'s `run_tonk_guide`. `TONK_SPOT` is always
/// removed first so the outer environment never leaks in; pass it
/// via `extra_env` to pin it explicitly.
fn tonk_cmd(state_dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(tonk_bin());
    cmd.args(args)
        .env("TONK_SPOTS_STATE", state_dir)
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_TELEMETRY")
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("TONK_UPDATE_STATE", state_dir)
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
