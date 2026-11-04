# Browser revocation of link-issued agent grants

Base: PR #1240 head `9c4a1a5aa8a5387ab71cc05622f4f34de532409e`.

Question: can the already-authorized browser device revoke a grant issued
directly by a link key to its agent, without opening account or agent custody?

Scope: tests only. Preserve the link-issued chain and its published grants.
Create the agent through the real repository creation code; recover only public
agent-to-account-to-device authority through the browser peer's proof search.
Sign the revocation with the device key, post it to a local access service,
and test the same hosted agent's read before and after. Do not delete grants.

Verification: `cargo test -p tonk-worker --lib --features integration-tests browser_cannot_revoke_link_issued_agent_grants_but_session_can`.

Result: complete. Device-signed artifact failed local verification with
`Unauthorized("the witness path does not show ... ever held ... capability")`.
The proof recovered from real repository creation has two hops; its first hop
is scoped specifically to the agent space, unlike the hosted agent-to-session
powerline. The service rejects the device's artifact as unauthorized, and the
same hosted writer can still read afterward.

Positive control: use the same revocation builder with the hosted session's
powerline instead. The service acknowledges each scoped grant's revocation;
the SAME writer's next read fails with `CREDENTIAL_REVOKED`. The agent's published
grant inventory still lists read access, proving that deletion is not the cause.
The account root, agent key and link key are out of scope during both attempts.
Only the positive control imports the hosted session seed, which is not held by
the browser. The browser never joins the target space.

The new focused integration test passed (0.69s). The existing
`the_account_revokes_an_agent_it_let_in` baseline also passed. Initial sandboxed
execution could not bind the local server; execution with loopback access worked.
The shell lacks cargo-nextest, so verification uses cargo test. Existing
deprecated-API and dead-code warnings remain untouched.

Limits: native execution of the browser worker's actual repository creation and
proof-search code against the real local access service, not a live browser UI
or WebAuthn journey. No production behavior changed. Current device authority
does not support this direct revocation path; a hosted-session management path
or a separately investigated change to delegated-recipient revocation would be
needed to make the no-passkey design work.
