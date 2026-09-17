# Connection view recovery audit

Date: 2026-09-17. Follow-up to the CLI connection implementation and the anonymous-space invitation regression.

## Scope and findings

Reviewed the changed agent invitation, onboarding prompt, invitation receipt, account registration return, terminal approval, connection management, and missing-space views. This is an audit of the connection/access surfaces, not every unrelated application view.

| Surface | Defect | Resolution |
| --- | --- | --- |
| Anonymous agent invite | Internal passkey error with a useless new-invite button | Explain account requirement and open account creation/sign-in; preserve the current space URL |
| Account verification | Generic sharing instructions and lost navigation | Invitation-specific verification copy; return to the original trusted space route |
| Local-only space | Invitation attempted before sync was enabled | Explain sync requirement; enable it only after the explicit turn-on-sync action |
| Invitation failures | Every failure offered the same action | Separate account, verification, sync, retry, new-link, busy, denied, and unavailable states |
| Invitation state transitions | Previous mode facts could survive | Publish the mode with cardinality one; regression asserts each transition |
| Ready invitation | Dense technical copy and misleading replacement implication | Short lowercase instructions; explain link access and that new invites do not cancel older ones |
| Theme support | Fixed light-theme colors | Use shared theme tokens for copy, recovery, and receipt controls |
| Agent receipt | Could imply online presence | Say agent setup confirmed; explicitly explain that this records a completed sync |
| Terminal approval after login | Login returned home and stranded CLI request | Preserve the signed request fragment through login/signup completion |
| Empty approval selection | No way to create a first space | Create-space action opens separately; refresh retains the approval request |
| Expired approval | Selection could erase expiry message | Share the expired state between timer and selection; disable approval controls |
| Connection management | Initial errors hid their own refresh action | Show actionable failure state; distinguish unsupported deployment from failed loading |
| Connection details | Raw identifiers and add-space errors in ordinary UI | Put identifiers behind details disclosures and use helpful permission copy |
| Missing space | Claimed invalid link when another account might have access | Explain account access and offer sign-in with trusted space return |

User-authored account and space names keep their original case. Technical diagnostics remain in logs; visible product instructions are lowercase. Account switching does not silently move spaces between accounts.

## Verification

- Registration Wasm browser suite: 13 passed, including full pathname/query/fragment return for account creation, verification, missing-space sign-in, and terminal approval.
- Standard-library suite: 34 passed, including actual library lowering, machine prompt preservation, receipt copy, and missing-space recovery markup.
- Worker invitation recovery suite: 3 passed. Exercises the same rootless space through account attachment, verification, explicit sync, and creation of an actual signed invitation. Also checks cached-link reuse, lost-link recovery, service-code classification, and exact mode transitions.
- Issuer permission suite: 3 passed. Distinguishes denied permission from unavailable proof storage and preserves the exact invitation expiry requirement without silently shortening it.
- Strict worker Clippy with `connection-invites`, library and tests, passed with warnings denied.
- UI Wasm compilation with `connection-invites`, formatting, and diff checks passed. Existing end-to-end copy assertions were updated; the full end-to-end suite was not rerun.
- Focused workspace browser regressions passed for selection/expiry, initial-load failure versus unsupported deployment, and connection management/details behavior.
- Isolated Chrome fixture using the actual recovery component: all nine modes expose only their intended action; account and verification clicks send the expected registration requests without guest-supplied return URLs. Dark desktop and light 320px screenshots inspected; no horizontal overflow in the narrow viewport. The registration bridge in this fixture is a stub.
- Real Web Awesome copy-button theme rendering was checked separately in light and dark mode during the preceding theme fix.

## Validation boundaries

The subsequent [CLI/browser E2E validation](004-connection-e2e-validation.md) exercises the account ceremonies with synthetic passkeys and records additional functional fixes. The following limits describe this original audit checkpoint.

No real user passkey, email activation, Safari, or deployed end-to-end ceremony was performed. Browser tests exercise host navigation and the backend fixture exercises account/space/invitation transitions, but those are not a live cross-device sign-in. The missing-space sign-in markup is library-tested and its host return behavior is browser-tested; its complete rendered guest-to-host click-through remains unverified. No local user storage or passkeys were cleared.
