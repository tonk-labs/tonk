//! CLI-level space resolution: spawns the real `tonk` binary against
//! an isolated registry (`TONK_SPACES_STATE`) and pins `TONK_SPACE` /
//! `--space` explicitly, so these exercise the same precedence and
//! error text a human or an agent actually sees — not just the
//! `space` module's in-process ops (covered by `tests/space.rs`).

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
/// otherwise write keys into the developer's real profile. `TONK_SPACE`
/// is always removed first so the outer environment never leaks in;
/// pass it via `extra_env` to pin it explicitly.
fn tonk_cmd(state_dir: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(tonk_bin());
    cmd.args(args)
        .env("TONK_SPACES_STATE", state_dir)
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_TELEMETRY")
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("TONK_UPDATE_STATE", state_dir)
        .env("HOME", state_dir)
        // These fixtures exercise remote selection, not identity provisioning.
        // Production omits this explicit unsafe compatibility override.
        .env("TONK_UNSAFE_ALLOW_DEVICE_ROOT", "1")
        .env_remove("TONK_SPACE");
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

/// Same isolation as [`tonk_cmd`], run *from* `cwd`. The binding
/// tier keys off the working directory, so these have to control it
/// rather than inherit the test runner's.
fn run_in(state_dir: &Path, cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
    tonk_cmd(state_dir, args, extra_env)
        .current_dir(cwd)
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

mod when_one_account_is_signed_in {
    use super::*;

    const ACCOUNT_A: &str = "did:key:z6MkAccountAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// Create a space through the CLI, then record `signed_in` as the
    /// account this installation is signed into.
    fn space_and_account(state: &Path, name: &str, signed_in: Option<&str>) {
        let site = state.join(format!("{name}-site"));
        let created = run(
            state,
            &[
                "space",
                "new",
                name,
                "--site",
                site.to_str().expect("utf-8 site"),
            ],
            &[],
        );
        assert!(created.status.success(), "{}", stderr_of(&created));
        let Some(signed_in) = signed_in else {
            return;
        };
        let path = state.join("spaces.json");
        let mut registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("registry"))
                .expect("registry JSON");
        registry["account"] = serde_json::json!({ "root": signed_in });
        std::fs::write(&path, serde_json::to_vec_pretty(&registry).expect("encode"))
            .expect("write registry");
    }

    /// The account slot parameterizes account-service operations and nothing
    /// else. A replica this device holds opens under any account, because the
    /// only enforcement that is real happens at the service boundary.
    #[dialog_common::test]
    fn a_space_this_account_did_not_create_still_opens() {
        let state = tempfile::tempdir().expect("tempdir");
        space_and_account(state.path(), "garden", Some(ACCOUNT_A));

        let output = run(state.path(), &["--space", "garden", "status"], &[]);

        assert!(output.status.success(), "{}", stderr_of(&output));
        let stderr = stderr_of(&output);
        assert!(!stderr.contains("doesn't have access"), "{stderr}");
    }

    /// …and it can still be written to. Editing is unrestricted.
    #[dialog_common::test]
    fn a_mutating_command_reaches_the_space_whatever_the_account() {
        let state = tempfile::tempdir().expect("tempdir");
        space_and_account(state.path(), "garden", Some(ACCOUNT_A));

        let output = run(
            state.path(),
            &["--space", "garden", "eval", "-c", "blank:"],
            &[],
        );

        assert!(output.status.success(), "{}", stderr_of(&output));
    }

    #[dialog_common::test]
    fn linking_without_an_account_says_to_sign_in_first() {
        let state = tempfile::tempdir().expect("tempdir");
        space_and_account(state.path(), "garden", None);

        let output = run(state.path(), &["space", "link", "garden"], &[]);

        assert!(!output.status.success());
        assert!(
            stderr_of(&output).contains("no account is signed in"),
            "{}",
            stderr_of(&output)
        );
    }

    /// The listing names the space and its owner, and has no access column:
    /// there is nothing for it to report, because nothing is refused.
    #[dialog_common::test]
    fn the_space_listing_carries_an_owner_and_no_access_column() {
        let state = tempfile::tempdir().expect("tempdir");
        space_and_account(state.path(), "garden", Some(ACCOUNT_A));

        let output = run(state.path(), &["space"], &[]);

        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("NAME"), "{stdout}");
        assert!(stdout.contains("OWNER"), "{stdout}");
        assert!(stdout.contains("ROLE"), "{stdout}");
        assert!(!stdout.contains("ACCESS"), "{stdout}");
        assert!(!stdout.contains("another account"), "{stdout}");
        // Local-only until it is linked: no roster, so no owner.
        assert!(stdout.contains("local"), "{stdout}");
    }
}

mod when_no_account_is_signed_in {
    use super::*;

    #[dialog_common::test]
    fn read_only_listing_writes_no_state() {
        let state = tempfile::tempdir().expect("tempdir");

        let spaces = run(state.path(), &["space"], &[]);
        assert!(spaces.status.success(), "{}", stderr_of(&spaces));
        assert!(stdout_of(&spaces).contains("no spaces registered"));

        assert!(!state.path().join("spaces.json").exists());
    }

    #[dialog_common::test]
    fn a_new_space_is_local_only_until_it_is_linked() {
        let state = tempfile::tempdir().expect("tempdir");
        let site = state.path().join("scratch-site");
        let output = run(
            state.path(),
            &[
                "space",
                "new",
                "scratch",
                "--site",
                site.to_str().expect("utf-8 site"),
            ],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));

        let spaces: serde_json::Value = serde_json::from_slice(
            &std::fs::read(state.path().join("spaces.json")).expect("space registry"),
        )
        .expect("spaces JSON");
        let entry = spaces["spaces"]["scratch"]
            .as_object()
            .expect("space entry");
        assert_eq!(
            entry.keys().collect::<Vec<_>>(),
            vec!["site"],
            "the registry records a binding and nothing more: {spaces}"
        );
        assert!(
            spaces["account"].is_null(),
            "no account is signed in: {spaces}"
        );
    }
}

/// A revocation relay for remotes that need one. Also on the discard port:
/// a mint parses this URL and embeds it in the link, and never calls it.
#[cfg(feature = "integration-tests")]
const DEAD_RELAY: &str = "http://127.0.0.1:9/revocations";

/// Stand up a real space through the CLI — `tonk space new` writes the
/// site the rest of these invocations read — then register `remotes`
/// in order. `tonk remote add` wires the *first* remote as `main`'s
/// upstream and leaves it alone after that, so `remotes[0]` is the one
/// the repo pushes to.
fn space_with_remotes(state_dir: &Path, remotes: &[(&str, &str)]) -> String {
    let relayed: Vec<(&str, &str, Option<&str>)> = remotes
        .iter()
        .map(|(name, endpoint)| (*name, *endpoint, None))
        .collect();
    space_with_relayed_remotes(state_dir, &relayed)
}

/// The same, with an explicit revocation relay per remote. A deployment no
/// longer advertises one and a mint no longer needs one, but a remote may
/// still be configured with a relay by hand, and that stays carried into the
/// link.
fn space_with_relayed_remotes(state_dir: &Path, remotes: &[(&str, &str, Option<&str>)]) -> String {
    let site = state_dir.join("site");
    let site = site.to_str().expect("utf-8 site path");
    let output = run(state_dir, &["space", "new", "demo", "--site", site], &[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let did = stdout_of(&output)
        .lines()
        .find_map(|line| line.strip_prefix("DID: "))
        .expect("space new reports the new space's DID")
        .to_owned();

    for (name, endpoint, relay) in remotes {
        let mut args = vec!["remote", "add", name, endpoint];
        if let Some(relay) = relay {
            args.extend_from_slice(&["--revocation-url", relay]);
        }
        let output = run(state_dir, &args, &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
    }
    did
}

/// Write a `spaces.json` registry directly (bypassing the CLI) so
/// each test starts from a known, isolated fixture. `entries` are
/// `(name, absolute site path)` pairs; site paths need not contain a
/// real repo — resolution and its error text are what's under test,
/// not site opening.
fn write_registry(state_dir: &Path, entries: &[(&str, &Path)], current: Option<&str>) {
    let spaces: String = entries
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
    let json = format!("{{{current}\"spaces\":{{{spaces}}}}}");
    std::fs::write(state_dir.join("spaces.json"), json).expect("write spaces.json");
}

mod when_nothing_is_registered {
    use super::*;

    #[dialog_common::test]
    fn bare_tonk_prints_the_same_index_as_help() {
        let state = tempfile::tempdir().expect("tempdir");
        let bare = run(state.path(), &[], &[]);
        let help = run(state.path(), &["-h"], &[]);
        assert!(bare.status.success(), "{}", stderr_of(&bare));
        assert!(help.status.success(), "{}", stderr_of(&help));
        assert_eq!(stdout_of(&bare), stdout_of(&help));
        assert!(stdout_of(&bare).contains("start a space"));
    }

    #[dialog_common::test]
    fn generic_assert_help_is_available_without_a_space() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["assert", "--help"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("query <CONCEPT> --json"), "{stdout}");
        assert!(
            stdout.contains("assert task <ENTITY> --done true"),
            "{stdout}"
        );
    }

    #[dialog_common::test]
    fn root_help_is_the_grouped_command_index() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["--help"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("examine state"), "{stdout}");
        assert!(stdout.contains("write facts"), "{stdout}");
        assert!(stdout.contains("collaborate"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_errors_with_space_new_hint_when_nothing_registered() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("no spaces registered"), "{stderr}");
        assert!(stderr.contains("--site"), "{stderr}");
    }
}

mod when_asking_for_unbind_help {
    use super::*;

    /// Clearing a dead binding (a directory that no longer
    /// exists) is `PATH`'s whole reason for being — canonicalization
    /// means only an absolute path can still match a vanished
    /// directory, so the help text has to say so.
    #[dialog_common::test]
    fn it_says_the_path_must_be_absolute() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["space", "unbind", "--help"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("absolute"), "{stdout}");
    }
}

mod when_resolving_with_precedence {
    use super::*;

    fn two_space_registry(state: &Path) {
        let a = state.join("site-a");
        let b = state.join("site-b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        write_registry(state, &[("a", &a), ("b", &b)], Some("b"));
    }

    #[dialog_common::test]
    fn it_prefers_the_flag_over_env() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(
            state.path(),
            &["--space", "a", "status"],
            &[("TONK_SPACE", "b")],
        );
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("active space: a (flag)"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_reads_tonk_space_from_the_environment() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(state.path(), &["status"], &[("TONK_SPACE", "a")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("active space: a (env)"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_accepts_canonical_space_flag_and_environment_names() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(state.path(), &["--space", "a", "status"], &[]);
        assert!(!output.status.success());
        assert!(stderr_of(&output).contains("active space: a (flag)"));

        let output = run(state.path(), &["status"], &[("TONK_SPACE", "a")]);
        assert!(!output.status.success());
        assert!(stderr_of(&output).contains("active space: a (env)"));
    }

    #[dialog_common::test]
    fn it_rejects_the_pre_rename_flag_and_environment_name() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        for args in [
            &["--spot", "a", "status"][..],
            &["spot", "link", "a"][..],
            &["account", "spots"][..],
        ] {
            let output = run(state.path(), args, &[]);
            assert!(!output.status.success());
            let stderr = stderr_of(&output);
            assert!(
                stderr.contains("a retired space command or option was supplied"),
                "{stderr}"
            );
            assert!(!stderr.to_ascii_lowercase().contains("spot"), "{stderr}");
        }

        let output = run(state.path(), &["status"], &[("TONK_SPOT", "a")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(
                "a retired space environment variable is set; unset it and use TONK_SPACE"
            ),
            "{stderr}"
        );
        assert!(!stderr.to_ascii_lowercase().contains("spot"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_rejects_the_pre_rename_state_directory_variable() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let mut cmd = Command::new(tonk_bin());
        cmd.args(["status"])
            .env_remove("TONK_SPACES_STATE")
            .env("TONK_SPOTS_STATE", state.path())
            .env("TONK_TELEMETRY_STATE", state.path())
            .env("DO_NOT_TRACK", "1")
            .env("TONK_NO_UPDATE_CHECK", "1")
            .env("HOME", state.path())
            .env("TONK_UNSAFE_ALLOW_DEVICE_ROOT", "1")
            .env("TONK_SPACE", "a");
        let output = cmd.output().expect("run tonk");
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(
                "a retired space environment variable is set; unset it and use TONK_SPACES_STATE"
            ),
            "{stderr}"
        );
        assert!(!stderr.to_ascii_lowercase().contains("spot"), "{stderr}");
    }

    #[dialog_common::test]
    fn canonical_and_pre_rename_environment_names_do_not_compete_silently() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(
            state.path(),
            &["status"],
            &[("TONK_SPACE", "a"), ("TONK_SPOT", "b")],
        );
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(
                "a retired space environment variable is set; unset it and use TONK_SPACE"
            ),
            "{stderr}"
        );
        assert!(!stderr.to_ascii_lowercase().contains("spot"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_does_not_fall_back_to_the_legacy_global_selection() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(state.path(), &["status"], &[("TONK_SPACE", "")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("no space active for this directory"),
            "{stderr}"
        );
    }

    #[dialog_common::test]
    fn bare_use_is_retired_in_favour_of_the_space_listing() {
        let state = tempfile::tempdir().expect("tempdir");
        two_space_registry(state.path());

        let output = run(state.path(), &["space", "use"], &[("TONK_SPACE", "a")]);
        assert!(!output.status.success());
        assert!(stderr_of(&output).contains("<NAME>"));
    }

    #[dialog_common::test]
    fn complete_help_uses_only_space_terminology() {
        let state = tempfile::tempdir().expect("tempdir");
        let output = run(state.path(), &["help", "--all"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(!stdout.to_ascii_lowercase().contains("spot"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_errors_on_an_unknown_space_naming_available() {
        let state = tempfile::tempdir().expect("tempdir");
        let a = state.path().join("site-a");
        std::fs::create_dir_all(&a).expect("mkdir a");
        write_registry(state.path(), &[("a", &a)], None);

        let output = run(state.path(), &["status"], &[("TONK_SPACE", "nope")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("unknown space 'nope'"), "{stderr}");
        assert!(stderr.contains("registered: a"), "{stderr}");
    }
}

mod when_using_space_agent_context {
    use super::*;

    #[dialog_common::test]
    fn it_explains_how_to_create_a_missing_claim() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);

        let output = run(state.path(), &["space", "agents"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("no AGENTS.md claim"), "{stderr}");
        assert!(
            stderr.contains("tonk space agents set AGENTS.md"),
            "{stderr}"
        );
    }

    #[dialog_common::test]
    fn it_round_trips_the_repository_claim_as_raw_markdown_and_json() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);
        let source = state.path().join("source.md");
        let expected = "# Demo space\n\n1. Run `tonk query task --json`.\n";
        std::fs::write(&source, expected).expect("write source");

        let output = run(
            state.path(),
            &[
                "space",
                "agents",
                "set",
                source.to_str().expect("utf-8 path"),
            ],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        let receipt = stdout_of(&output);
        assert!(receipt.contains("asserted AGENTS.md claim"), "{receipt}");
        assert!(receipt.contains("entity: did:key:"), "{receipt}");
        assert!(receipt.contains("revision:"), "{receipt}");

        let output = run(state.path(), &["space", "agents"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert_eq!(stdout_of(&output), expected);

        let output = run(state.path(), &["space", "agents", "get", "--json"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("valid claim JSON");
        assert_eq!(value["schemaVersion"], "tonk.agents-get.v1");
        assert_eq!(value["rows"][0]["source"], "dialog-claim");
        assert_eq!(value["rows"][0]["attribute"], "xyz.tonk.repo/agents");
        assert_eq!(value["rows"][0]["markdown"], expected);
        assert!(
            value["rows"][0]["entity"]
                .as_str()
                .is_some_and(|entity| entity.starts_with("did:key:"))
        );
        assert!(
            value["rows"][0]["revision"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[dialog_common::test]
    fn quiet_agents_set_suppresses_the_claim_rows() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);
        let source = state.path().join("source.md");
        std::fs::write(&source, "# Demo space\n").expect("write source");

        let output = run(
            state.path(),
            &[
                "space",
                "agents",
                "set",
                source.to_str().expect("utf-8 path"),
                "--quiet",
            ],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert_eq!(stdout_of(&output), "asserted AGENTS.md claim\n");
    }

    #[dialog_common::test]
    fn no_sync_on_a_noun_write_does_not_touch_the_remote() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[("origin", DEAD_REMOTE)]);
        let source = state.path().join("source.md");
        std::fs::write(&source, "# Demo space\n").expect("write source");

        let output = run(
            state.path(),
            &[
                "space",
                "agents",
                "set",
                source.to_str().expect("utf-8 path"),
                "--no-sync",
            ],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        assert!(
            !stderr_of(&output).contains("auto-sync"),
            "{}",
            stderr_of(&output)
        );
    }
}

mod when_importing {
    use super::*;

    #[dialog_common::test]
    fn dry_run_reads_the_csv_without_committing_it() {
        let state = tempfile::tempdir().expect("tempdir");
        let created = run(state.path(), &["space", "new", "demo"], &[]);
        assert!(created.status.success(), "{}", stderr_of(&created));
        let csv = state.path().join("export.csv");
        let exported = run(
            state.path(),
            &["export", "--out", csv.to_str().expect("utf-8 export path")],
            &[("TONK_SPACE", "demo")],
        );
        assert!(exported.status.success(), "{}", stderr_of(&exported));
        let before = run(
            state.path(),
            &["status", "--json"],
            &[("TONK_SPACE", "demo")],
        );
        let before: serde_json::Value =
            serde_json::from_slice(&before.stdout).expect("status JSON before");

        let imported = run(
            state.path(),
            &[
                "import",
                csv.to_str().expect("utf-8 export path"),
                "--dry-run",
            ],
            &[("TONK_SPACE", "demo")],
        );
        assert!(imported.status.success(), "{}", stderr_of(&imported));
        assert!(stdout_of(&imported).contains("dry run"));

        let after = run(
            state.path(),
            &["status", "--json"],
            &[("TONK_SPACE", "demo")],
        );
        let after: serde_json::Value =
            serde_json::from_slice(&after.stdout).expect("status JSON after");
        assert_eq!(before["sync"]["hash"], after["sync"]["hash"]);
    }

    #[dialog_common::test]
    fn no_sync_import_does_not_touch_the_configured_remote() {
        let state = tempfile::tempdir().expect("tempdir");
        let created = run(state.path(), &["space", "new", "demo"], &[]);
        assert!(created.status.success(), "{}", stderr_of(&created));
        let csv = state.path().join("export.csv");
        let exported = run(
            state.path(),
            &["export", "--out", csv.to_str().expect("utf-8 path")],
            &[("TONK_SPACE", "demo")],
        );
        assert!(exported.status.success(), "{}", stderr_of(&exported));
        let remote = run(
            state.path(),
            &["remote", "add", "origin", DEAD_REMOTE],
            &[("TONK_SPACE", "demo")],
        );
        assert!(remote.status.success(), "{}", stderr_of(&remote));

        let imported = run(
            state.path(),
            &["import", csv.to_str().expect("utf-8 path"), "--no-sync"],
            &[("TONK_SPACE", "demo")],
        );
        assert!(imported.status.success(), "{}", stderr_of(&imported));
        assert!(
            !stderr_of(&imported).contains("auto-sync"),
            "{}",
            stderr_of(&imported)
        );
    }
}

#[dialog_common::test]
fn status_reports_when_a_configured_remote_cannot_be_fetched() {
    let state = tempfile::tempdir().expect("tempdir");
    space_with_remotes(state.path(), &[("origin", DEAD_REMOTE)]);

    let started = std::time::Instant::now();
    let output = run(
        state.path(),
        &["status", "--json"],
        &[("TONK_SPACE", "demo")],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "failed status fetch took {:?}",
        started.elapsed()
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
    assert_eq!(value["schemaVersion"], "tonk.status.v2");
    assert_eq!(value["sync"]["state"], "not-fetched");
    assert_eq!(value["sync"]["fetched"], false);
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
        space_with_remotes(
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

/// A relay-less remote is no longer a reason to refuse a mint. A revocation
/// is an ordinary `ucan/revoke` invocation now, addressed to the access
/// service the invite already carries, so there is nothing extra for the
/// link to name. The refusal this replaces is the one that used to demand
/// `tonk remote add --revocation-url`.
mod when_a_remote_carries_no_revocation_relay {
    use super::*;

    /// The command still exits non-zero: past this point it pulls and pushes,
    /// and the upstream is a discard port — the same reason the other offline
    /// mints assert on stderr and never on success. That the refusal is absent
    /// is meaningful anyway, because the check ran *before* the network and
    /// would otherwise be the first line printed.
    #[dialog_common::test]
    fn it_mints_an_invite_that_embeds_it() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[("origin", DEAD_REMOTE)]);

        let output = run(state.path(), &["invite"], &[]);
        let stderr = stderr_of(&output);
        assert!(
            !stderr.contains("no revocation relay"),
            "a relay is not required to mint: {stderr}"
        );
        assert!(
            !stderr.contains("--revocation-url"),
            "and nothing sends the reader to configure one: {stderr}"
        );
    }
}

mod when_the_invite_remote_is_the_upstream {
    use super::*;

    #[dialog_common::test]
    fn it_mints_without_a_mismatch_warning() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(
            state.path(),
            &[("origin", DEAD_REMOTE), ("other", OTHER_DEAD_REMOTE)],
        );

        let output = run(state.path(), &["invite", "--remote", "origin"], &[]);
        let stderr = stderr_of(&output);
        assert!(!stderr.contains("the invite embeds remote"), "{stderr}");
    }
}

/// A space with no remote is served by no deployment, so an invite to
/// it has no origin. The canonical base is not a fallback here — it is
/// production, which holds none of this space's data — so the mint is
/// refused rather than handed over as a link that reads fine and works
/// for nobody.
///
/// `--base-url` is the way through for a deployment tonk doesn't know
/// about. `--no-shorten` is what makes *that* testable: shortening
/// PUTs to the link's own origin, so without it these would reach the
/// wire on every run.
mod when_no_remote_is_registered_at_all {
    use super::*;

    const UNREGISTERED_BASE: &str = "http://127.0.0.1:9/join";

    #[dialog_common::test]
    fn it_refuses_rather_than_minting_a_link_to_production() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);

        let output = run(state.path(), &["invite", "--no-shorten"], &[]);

        assert!(!output.status.success(), "{}", stdout_of(&output));
        assert!(
            stdout_of(&output).trim().is_empty(),
            "no link should reach stdout: {}",
            stdout_of(&output)
        );
        let stderr = stderr_of(&output);
        assert!(stderr.contains("'demo' has no remote"), "{stderr}");
        assert!(
            stderr.contains(tonk_cli::invite::DEFAULT_BASE_URL),
            "the refusal names the origin it would otherwise have used: {stderr}"
        );
    }

    /// The refusal is only useful if it names every way out: hand the
    /// space to an account, register a remote, or override the origin.
    #[dialog_common::test]
    fn it_names_each_way_to_give_the_space_an_origin() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);

        let output = run(state.path(), &["invite", "--no-shorten"], &[]);
        let stderr = stderr_of(&output);

        assert!(stderr.contains("tonk account login"), "{stderr}");
        assert!(stderr.contains("tonk space link demo"), "{stderr}");
        assert!(stderr.contains("tonk remote add"), "{stderr}");
        assert!(stderr.contains("--base-url"), "{stderr}");
    }

    /// An explicit `--base-url` is the caller saying which deployment
    /// serves this space, so the mint proceeds on that origin.
    #[dialog_common::test]
    fn it_mints_on_an_explicitly_named_base() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);

        let output = run(
            state.path(),
            &["invite", "--no-shorten", "--base-url", UNREGISTERED_BASE],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));

        let url = stdout_of(&output);
        let url = url.trim();
        assert!(url.starts_with(UNREGISTERED_BASE), "named base: {url}");
        assert!(url.contains("access="), "a real invite: {url}");
    }

    /// The environment form of `--no-shorten`, so automation can opt
    /// out without threading a flag through every call site.
    #[dialog_common::test]
    fn it_honours_the_environment_switch() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(state.path(), &[]);

        let output = run(
            state.path(),
            &["invite", "--base-url", UNREGISTERED_BASE],
            &[("TONK_NO_SHORTEN", "1")],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));

        let url = stdout_of(&output);
        let url = url.trim();
        assert!(url.starts_with(UNREGISTERED_BASE), "named base: {url}");
        assert!(!url.contains("/@/"), "not shortened: {url}");
    }
}

mod when_inviting_without_a_remote {
    use super::*;

    #[dialog_common::test]
    fn it_mints_without_a_mismatch_warning() {
        let state = tempfile::tempdir().expect("tempdir");
        space_with_remotes(
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
        space_with_remotes(
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
        space_with_remotes(
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

    /// Set a space up and run one `tonk` invocation against it, all on
    /// the blocking pool. `#[dialog_common::test]` expands to
    /// `#[tokio::test]`, whose current-thread runtime is also hosting
    /// the access service — block that thread on a subprocess and the
    /// service the CLI is talking to can never answer.
    async fn mint_against(
        env: &AccessServiceAddress,
        state_dir: PathBuf,
        endpoint: String,
        args: &[&str],
    ) -> Output {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let setup_dir = state_dir.clone();
        let setup_endpoint = endpoint.clone();
        // Create the space first so its DID can be provisioned: minting
        // pushes, and the access service serves no unprovisioned space.
        let did = tokio::task::spawn_blocking(move || {
            // With a relay, to keep the hand-configured path exercised
            // end-to-end against a live service.
            space_with_relayed_remotes(&setup_dir, &[("origin", &setup_endpoint, Some(DEAD_RELAY))])
        })
        .await
        .expect("blocking space setup joins");
        env.provision_subject(&did)
            .await
            .expect("the space provisions");

        tokio::task::spawn_blocking(move || {
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

        let output = mint_against(
            &env,
            state.path().to_path_buf(),
            origin.clone(),
            &["invite"],
        )
        .await;
        assert!(output.status.success(), "{}", stderr_of(&output));

        let stdout = stdout_of(&output);
        let url = stdout.trim();
        assert!(
            url.starts_with(&format!("{origin}/@/")),
            "short link sits on the remote's origin: {url}"
        );
        assert!(!url.contains("tonk.network"), "not the default base: {url}");
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
            &env,
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
        assert!(!url.contains("tonk.network"), "not the default base: {url}");

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
            &env,
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

#[cfg(feature = "integration-tests")]
mod when_status_is_synced {
    use anyhow::Result;
    use tonk_access_service::helpers::AccessServiceAddress;

    use super::*;

    #[dialog_common::test]
    async fn it_includes_the_current_hash(env: AccessServiceAddress) -> Result<()> {
        let state = tempfile::tempdir()?;
        let state_dir = state.path().to_path_buf();
        let endpoint = env.access_service_url.trim_end_matches('/').to_owned();

        let setup_dir = state_dir.clone();
        let setup_endpoint = endpoint.clone();
        let did = tokio::task::spawn_blocking(move || {
            space_with_remotes(&setup_dir, &[("origin", &setup_endpoint)])
        })
        .await
        .expect("blocking space setup joins");
        env.provision_subject(&did)
            .await
            .expect("the space provisions");

        let (expected_hash, status) = tokio::task::spawn_blocking(move || {
            let pushed = run(&state_dir, &["push"], &[]);
            assert!(pushed.status.success(), "{}", stderr_of(&pushed));
            let pushed = stdout_of(&pushed);
            let expected_hash = pushed
                .lines()
                .find_map(|line| line.strip_prefix("after:  "))
                .expect("push reports the current hash")
                .to_owned();

            let status = run(&state_dir, &["status"], &[]);
            assert!(status.status.success(), "{}", stderr_of(&status));
            (expected_hash, stdout_of(&status))
        })
        .await
        .expect("blocking tonk invocations join");

        assert!(status.contains("sync: synced\n"), "{status}");
        assert!(
            status.contains(&format!("hash: {expected_hash}")),
            "current hash {expected_hash} missing from status output:\n{status}"
        );
        Ok(())
    }
}

mod when_a_directory_is_bound {
    use super::*;

    /// Two registered spaces plus a real `work/nested/` tree to bind
    /// and run from. Site paths need
    /// not hold a repo: resolution and its error text are what is
    /// under test, not site opening.
    fn fixture(state: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let a = state.join("site-a");
        let b = state.join("site-b");
        std::fs::create_dir_all(&a).expect("mkdir a");
        std::fs::create_dir_all(&b).expect("mkdir b");
        write_registry(state, &[("a", &a), ("b", &b)], Some("b"));

        let work = state.join("work");
        let nested = work.join("nested");
        std::fs::create_dir_all(&nested).expect("mkdir work/nested");
        (work, nested)
    }

    /// The CLI stores canonicalized paths, and on macOS a tempdir
    /// under `/var/...` canonicalizes to `/private/var/...`, so
    /// assertions on printed paths have to canonicalize too.
    fn shown(path: &Path) -> String {
        path.canonicalize()
            .expect("canonicalize")
            .display()
            .to_string()
    }

    fn bind(state: &Path, cwd: &Path, name: &str) {
        let output = run_in(state, cwd, &["space", "use", name], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
    }

    #[dialog_common::test]
    fn it_resolves_the_binding_from_a_subdirectory() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let output = run_in(state.path(), &nested, &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("active space: a (directory"), "{stderr}");
        assert!(stderr.contains(&shown(&work)), "{stderr}");
    }

    #[dialog_common::test]
    fn it_has_no_selection_outside_a_binding() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let elsewhere = state.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("mkdir elsewhere");
        let output = run_in(state.path(), &elsewhere, &["space"], &[]);
        let stdout = stdout_of(&output);
        assert!(!stdout.contains("active here:"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_prefers_tonk_space_over_a_binding() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let output = run_in(state.path(), &nested, &["status"], &[("TONK_SPACE", "b")]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("active space: b (env)"), "{stderr}");
    }

    #[dialog_common::test]
    fn use_reports_a_process_override_separately_from_the_binding() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());

        let output = run_in(
            state.path(),
            &work,
            &["space", "use", "a"],
            &[("TONK_SPACE", "b")],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("binding: a"), "{stdout}");
        assert!(stdout.contains("active space: b (env)"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_takes_the_deepest_binding() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        bind(state.path(), &work, "a");
        bind(state.path(), &nested, "b");

        let output = run_in(state.path(), &nested, &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("active space: b (directory"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_lists_bindings() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let output = run_in(state.path(), state.path(), &["space"], &[]);
        let stdout = stdout_of(&output);
        assert!(stdout.contains("directories:"), "{stdout}");
        assert!(stdout.contains(&shown(&work)), "{stdout}");
    }

    #[dialog_common::test]
    fn it_refuses_to_unbind_from_a_subdirectory() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let refused = run_in(state.path(), &nested, &["space", "unbind"], &[]);
        assert!(!refused.status.success());
        let stderr = stderr_of(&refused);
        assert!(stderr.contains("is bound to a"), "{stderr}");

        let unbound = run_in(state.path(), &work, &["space", "unbind"], &[]);
        assert!(unbound.status.success(), "{}", stderr_of(&unbound));

        let output = run_in(state.path(), &nested, &["status"], &[]);
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("no space active for this directory"),
            "{stderr}"
        );
    }

    #[dialog_common::test]
    fn it_reports_the_previous_binding_on_reattach() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let output = run_in(state.path(), &work, &["space", "use", "b"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("binding: b (was a)"), "{stdout}");
        assert!(stdout.contains("active space: b (directory"), "{stdout}");
    }

    #[dialog_common::test]
    fn it_rebinds_the_directory_immediately() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        // `use` rewrites this directory's one binding.
        let output = run_in(state.path(), &work, &["space", "use", "b"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("binding: b (was a)"), "{stdout}");
        assert!(stdout.contains("active space: b (directory"), "{stdout}");

        let status = run_in(state.path(), &work, &["status"], &[]);
        let stderr = stderr_of(&status);
        assert!(stderr.contains("active space: b (directory"), "{stderr}");
    }

    #[dialog_common::test]
    fn it_binds_without_a_shadow_warning() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());

        let output = run_in(state.path(), &work, &["space", "use", "a"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stderr = stderr_of(&output);
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[dialog_common::test]
    fn space_new_rebinds_the_invocation_directory() {
        let state = tempfile::tempdir().expect("tempdir");
        let (work, _nested) = fixture(state.path());
        bind(state.path(), &work, "a");

        let site = state.path().join("site-c");
        let site = site.to_str().expect("utf-8 site path");
        let output = run_in(
            state.path(),
            &work,
            &["space", "new", "c", "--site", site],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("active space: c (directory"), "{stdout}");

        let status = run_in(state.path(), &work, &["status"], &[]);
        assert!(status.status.success(), "{}", stderr_of(&status));
        let stdout = stdout_of(&status);
        assert!(stdout.contains("space: c\n"), "{stdout}");
        assert!(stdout.contains("selected via: directory"), "{stdout}");
    }
}

mod when_a_binding_is_orphaned {
    use super::*;

    /// A directory bound to a name that isn't registered — the
    /// hand-edited-`spaces.json` scenario `space rm`'s own pruning
    /// normally prevents. The resulting error must say the name came
    /// from the binding and point at `space unbind`, not read as
    /// an unexplained `unknown space`.
    #[dialog_common::test]
    fn it_blames_the_binding_in_the_unknown_space_error() {
        let state = tempfile::tempdir().expect("tempdir");
        let b = state.path().join("site-b");
        std::fs::create_dir_all(&b).expect("mkdir b");
        let work = state.path().join("work");
        std::fs::create_dir_all(&work).expect("mkdir work");
        let work_canon = work.canonicalize().expect("canonicalize work");

        let json = format!(
            "{{\"current\":\"b\",\"spaces\":{{\"b\":{{\"site\":{site:?}}}}},\
             \"bindings\":{{{dir:?}:\"a\"}}}}",
            site = b.display().to_string(),
            dir = work_canon.display().to_string(),
        );
        std::fs::write(state.path().join("spaces.json"), json).expect("write spaces.json");

        let output = run_in(state.path(), &work, &["status"], &[]);
        assert!(!output.status.success());
        let stderr = stderr_of(&output);
        assert!(stderr.contains("unknown space 'a'"), "{stderr}");
        assert!(
            stderr.contains(&work_canon.display().to_string()),
            "{stderr}"
        );
        assert!(stderr.contains("space unbind"), "{stderr}");
    }
}

/// `tonk space rm` is the only command that destroys facts, so its
/// guard rails are worth exercising through the real binary: the
/// prompt, the non-interactive refusal, and what each mode leaves on
/// disk. `run` pipes stdin, so every invocation here is exactly the
/// non-terminal case a script would hit.
mod when_deleting_a_space {
    use super::*;

    /// Create a space at its canonical path, bound to `cwd`, and
    /// return where its data landed.
    fn canonical_space(state_dir: &Path, cwd: &Path, name: &str) -> std::path::PathBuf {
        let output = run_in(state_dir, cwd, &["space", "new", name], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let site = state_dir.join("spaces").join(name);
        assert!(site.is_dir(), "site data at {}", site.display());
        site
    }

    fn work_dir(state: &tempfile::TempDir) -> std::path::PathBuf {
        let work = state.path().join("work");
        std::fs::create_dir_all(&work).expect("mkdir work");
        work.canonicalize().expect("canonicalize work")
    }

    #[dialog_common::test]
    fn it_refuses_to_delete_when_the_prompt_cannot_be_answered() {
        let state = tempfile::tempdir().expect("tempdir");
        let work = work_dir(&state);
        let site = canonical_space(state.path(), &work, "garden");

        let output = run_in(state.path(), &work, &["space", "rm", "garden"], &[]);
        assert!(!output.status.success(), "{}", stdout_of(&output));
        let stderr = stderr_of(&output);
        assert!(stderr.contains("stdin is not a terminal"), "{stderr}");
        assert!(stderr.contains("--yes"), "{stderr}");
        assert!(stderr.contains("--keep-data"), "{stderr}");

        // Nothing moved: the space is still registered and its data
        // is untouched.
        assert!(site.is_dir(), "data survives the refusal");
        let listed = stdout_of(&run(state.path(), &["space"], &[]));
        assert!(listed.contains("garden"), "{listed}");
    }

    #[dialog_common::test]
    fn it_deletes_the_data_with_yes() {
        let state = tempfile::tempdir().expect("tempdir");
        let work = work_dir(&state);
        let site = canonical_space(state.path(), &work, "garden");

        let output = run_in(
            state.path(),
            &work,
            &["space", "rm", "garden", "--yes"],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(
            stdout.contains("Deleted space 'garden' and its data"),
            "{stdout}"
        );
        assert!(!site.exists(), "data deleted");

        let listed = stdout_of(&run(state.path(), &["space"], &[]));
        assert!(listed.contains("no spaces registered"), "{listed}");
        assert!(!listed.contains("unregistered site data"), "{listed}");
    }

    /// The old default, now explicit. What matters is that the data
    /// it leaves behind stops being invisible.
    #[dialog_common::test]
    fn it_reports_kept_data_as_unregistered_and_re_adoptable() {
        let state = tempfile::tempdir().expect("tempdir");
        let work = work_dir(&state);
        let site = canonical_space(state.path(), &work, "garden");

        let output = run_in(
            state.path(),
            &work,
            &["space", "rm", "garden", "--keep-data"],
            &[],
        );
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(stdout.contains("Unregistered space 'garden'"), "{stdout}");
        assert!(stdout.contains("data kept at"), "{stdout}");
        assert!(site.is_dir(), "data kept");

        let listed = stdout_of(&run(state.path(), &["space"], &[]));
        assert!(listed.contains("unregistered site data"), "{listed}");
        let canonical = site.canonicalize().expect("canonicalize site");
        assert!(
            listed.contains(&canonical.display().to_string()),
            "{listed}"
        );
    }

    /// Re-using the name after `--keep-data` silently picks the old
    /// facts back up. Useful, but it must say so — otherwise a space
    /// that was just "removed" comes back full of data.
    #[dialog_common::test]
    fn it_says_when_a_new_space_adopted_leftover_data() {
        let state = tempfile::tempdir().expect("tempdir");
        let work = work_dir(&state);
        canonical_space(state.path(), &work, "garden");
        let removed = run_in(
            state.path(),
            &work,
            &["space", "rm", "garden", "--keep-data"],
            &[],
        );
        assert!(removed.status.success(), "{}", stderr_of(&removed));

        let output = run_in(state.path(), &work, &["space", "new", "garden"], &[]);
        assert!(output.status.success(), "{}", stderr_of(&output));
        let stdout = stdout_of(&output);
        assert!(
            stdout.contains("on the site data already at that path"),
            "{stdout}"
        );
    }
}

/// Every listing verb speaks the one format, and every read verb has a
/// machine-readable form.
///
/// These are pinned end to end through the binary rather than through
/// `listing::Listing`, because the thing that regressed before was not the
/// renderer — it was seven call sites each choosing whether to use one.
mod when_reading {
    use super::*;

    /// A space with something in every listing this can populate without
    /// a network or a file on disk.
    ///
    /// `remote` is separate because registering one also wires it as the
    /// upstream, after which `tonk status` reaches for the wire — and the
    /// remote here has nothing behind it.
    fn populated(state: &Path, remote: bool) {
        let run = |args: &[&str]| {
            let out = run(state, args, &[("TONK_SPACE", "demo")]);
            assert!(out.status.success(), "{args:?} failed: {}", stderr_of(&out));
        };
        run(&["space", "new", "demo"]);
        run(&["concept", "add", "task", "--field", "title:text:one"]);
        run(&["view", "add", "task", "--template", "<b>{title}</b>"]);
        let agents = state.join("AGENTS.md");
        std::fs::write(&agents, "# Demo space\n").expect("write AGENTS.md");
        run(&[
            "space",
            "agents",
            "set",
            agents.to_str().expect("utf-8 path"),
        ]);
        if remote {
            run(&["remote", "add", "origin", DEAD_REMOTE]);
        }
    }

    const LISTINGS: [&[&str]; 5] = [&["concept"], &["view"], &["blob"], &["remote"], &["space"]];

    /// An empty listing is one parenthesised sentence, not silence. The
    /// silent ones read as a broken command to anyone who ran them before
    /// there was anything to see.
    #[dialog_common::test]
    fn it_says_so_when_a_listing_is_empty() {
        let state = tempfile::tempdir().expect("tempdir");
        let created = run(state.path(), &["space", "new", "demo"], &[]);
        assert!(created.status.success(), "{}", stderr_of(&created));

        for args in LISTINGS {
            let out = run(state.path(), args, &[("TONK_SPACE", "demo")]);
            let stdout = stdout_of(&out);
            if args == ["space"] {
                // The one listing that is never empty: the space just
                // created is in it.
                continue;
            }
            assert!(
                stdout.starts_with('(') && stdout.trim_end().ends_with(')'),
                "{args:?} should print one parenthesised line when empty, saw:\n{stdout}"
            );
        }
    }

    /// One header row, tab separated, with the same column count on every
    /// row under it.
    #[dialog_common::test]
    fn it_heads_every_populated_listing_with_its_columns() {
        let state = tempfile::tempdir().expect("tempdir");
        populated(state.path(), true);

        for args in LISTINGS {
            if args == ["blob"] {
                continue; // nothing ingests a blob without a file to read
            }
            let out = run(state.path(), args, &[("TONK_SPACE", "demo")]);
            let stdout = stdout_of(&out);
            let mut lines = stdout.lines();
            let header = lines.next().unwrap_or_default();
            assert!(
                header.split('\t').all(|c| c == c.to_uppercase()),
                "{args:?} should lead with an upper-case header, saw:\n{stdout}"
            );
            let columns = header.split('\t').count();
            for row in lines.take_while(|line| !line.is_empty()) {
                assert_eq!(
                    row.split('\t').count(),
                    columns,
                    "{args:?} row {row:?} does not match its header {header:?}"
                );
            }
        }
    }

    /// The gap this closes was four of thirteen reads, so the guard is
    /// the whole list rather than the ones that happened to get one.
    #[dialog_common::test]
    fn it_offers_json_on_every_read() {
        let state = tempfile::tempdir().expect("tempdir");
        populated(state.path(), false);

        for args in [
            vec!["status", "--json"],
            vec!["account", "status", "--json"],
            vec!["space", "agents", "get", "--json"],
            vec!["query", "task", "--json"],
            vec!["concept", "--json"],
            vec!["view", "--json"],
            vec!["blob", "--json"],
            vec!["space", "--json"],
        ] {
            let out = run(state.path(), &args, &[("TONK_SPACE", "demo")]);
            assert!(out.status.success(), "{args:?} failed: {}", stderr_of(&out));
            let stdout = stdout_of(&out);
            let document: serde_json::Value = serde_json::from_str(&stdout)
                .unwrap_or_else(|e| panic!("{args:?} did not emit JSON ({e}):\n{stdout}"));

            // One envelope for every read. `tonk query` is the exception
            // and says so: it emits an EvaluateResponse, the same shape
            // `tonk eval --json` emits, which is a transaction
            // envelope rather than a listing.
            if args[0] != "query" {
                let version = document["schemaVersion"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{args:?} carries no schemaVersion:\n{stdout}"));
                assert!(
                    version.starts_with("tonk.") && version.contains(".v"),
                    "{args:?} schemaVersion {version:?} is not tonk.<command>.v<n>"
                );
            }
            if matches!(
                args.as_slice(),
                ["space", "agents", "get", "--json"]
                    | ["concept", "--json"]
                    | ["view", "--json"]
                    | ["blob", "--json"]
                    | ["space", "--json"]
            ) {
                assert!(
                    document["rows"].is_array(),
                    "{args:?} carries no rows: {stdout}"
                );
            }
        }

        // Separate state: registering a remote wires the upstream, which
        // is what `status` above must not have.
        let with_remote = tempfile::tempdir().expect("tempdir");
        populated(with_remote.path(), true);
        let out = run(
            with_remote.path(),
            &["remote", "--json"],
            &[("TONK_SPACE", "demo")],
        );
        assert!(out.status.success(), "{}", stderr_of(&out));
        let stdout = stdout_of(&out);
        let document: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("remote --json ({e}):\n{stdout}"));
        assert_eq!(document["schemaVersion"], "tonk.remote-list.v1", "{stdout}");
        assert_eq!(document["rows"][0]["name"], "origin", "{stdout}");
        // The version used to be repeated on every row, saying something
        // true of the whole response once per element.
        assert!(document["rows"][0]["version"].is_null(), "{stdout}");
    }

    #[dialog_common::test]
    fn show_dispatches_schema_concept_view_and_entity_names() {
        let state = tempfile::tempdir().expect("tempdir");
        populated(state.path(), false);
        let env = &[("TONK_SPACE", "demo")];

        let schema = run(state.path(), &["show"], env);
        assert!(schema.status.success(), "{}", stderr_of(&schema));
        assert!(stdout_of(&schema).contains("concept!: &task"));

        let concept = run(state.path(), &["show", "task", "--json"], env);
        assert!(concept.status.success(), "{}", stderr_of(&concept));
        let concept: serde_json::Value =
            serde_json::from_str(&stdout_of(&concept)).expect("concept JSON");
        assert_eq!(concept["schemaVersion"], "tonk.show-concept.v1");
        assert_eq!(concept["fields"][0]["name"], "title");
        assert!(concept["recipes"].as_array().is_some_and(|recipes| {
            recipes
                .iter()
                .any(|recipe| recipe == "tonk assert task --<field> <value>")
        }));

        // A view now lives ON the model entity, so `show task` above is
        // also the view's home; the template itself is listed by
        // `tonk view` (covered in the views suite).

        let seeded = run(
            state.path(),
            &["eval", "-c", "task!: &first\n  title: \"First\""],
            env,
        );
        assert!(seeded.status.success(), "{}", stderr_of(&seeded));
        let entity = run(state.path(), &["show", "first", "--json"], env);
        assert!(entity.status.success(), "{}", stderr_of(&entity));
        let entity: serde_json::Value =
            serde_json::from_str(&stdout_of(&entity)).expect("entity JSON");
        assert_eq!(entity["schemaVersion"], "tonk.show-entity.v1");
        assert!(entity["facts"].as_array().is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact["attribute"] == "xyz.tonk.task/title"
                    && fact["value"]
                        .as_str()
                        .is_some_and(|value| value.contains("First"))
            })
        }));
    }
}
