# Cross-deployment account branch restoration

The callback installed a grant on the signed-out branch without using the retained account branch. Restore parity with `custody::complete_login`.

- Validate the delivery and grant before selecting the issuer's branch.
- Release the callback read lock before `profiles::for_account`, then hold its selected-branch guard through installation and link completion.
- Exercise repeated sign-out/re-login with an unsynced directory entry, and reject a wrong-audience grant without activating its issuer's retained branch.
- Validation passed: 11 Wasm callback tests, 23 existing Wasm profile-routing tests, six native callback tests, `cargo fmt --all -- --check`, and `git diff --check`.
- The first runner attempt failed to start its daemon under the sandbox; the same artifact ran with local execution access. The first regression fixture omitted the branch record normally written during initial link completion; recording that state explicitly fixed the fixture, and both restoration and invalid-grant tests passed.
- Existing warnings in unchanged worker code remain.
- The full cross-deployment browser/passkey journey is outside this focused regression run.
