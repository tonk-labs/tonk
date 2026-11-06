# WKWebView passkey spike

**Result: successful real passkey assertion with both 32-byte PRF outputs.**
The user approved the ceremony in the installed probe on 2026-10-06; fresh
accessibility inspection confirmed the success message. No account was changed.

The integrated `Tonk Login Test.app` subsequently completed real account login
in an isolated WebKit profile. A manual catalog Refresh restored 14 spaces;
Tonk Town Scratch rendered successfully and reported sync: idle. Automatic
catalog refresh after background sync, cancellation and relaunch remain open
validation items; see `plans/038-direct-passkey-login.md`.

A separate SwiftUI app tests a native button calling WebKit's WebAuthn API on
`https://tonk.network`. It uses an ephemeral web data store and never logs in,
registers a passkey, changes account custody, or uses the desktop session.
Only success/error categories return to Swift. Both PRF outputs must be 32 bytes;
the script clears its output buffers after checking them.

The test bundle intentionally uses `xyz.tonk` to exercise the real association.
Do not install it over Tonk. Launch the app at the generated path explicitly.

## Build

```sh
TONK_SIGN_IDENTITY='Developer ID Application: Tonk Labs LTD (8WVKS2F24C)' \
  bash experiments/wk-passkey/build.sh
```

Without a provisioning profile this builds the unassociated baseline. For the
associated test, additionally set `TONK_PROVISIONING_PROFILE` to the downloaded
Tonk Labs Developer ID profile for `xyz.tonk`. The script checks that the profile
permits Associated Domains, embeds it, and derives a narrow entitlement file and
the matching AASA JSON in `.build/wk-passkey/`. The matching AASA file must be
served as JSON at `https://tonk.network/.well-known/apple-app-site-association`.
Preserve any other associations already published there.

## Observed on 2026-10-06

- Compiled with installed Xcode SDK for deployment target macOS 15.
- Signed and strictly verified using Tonk Labs team `8WVKS2F24C`.
- Launched on macOS 27.0, loaded Tonk in the attached WKWebView.
- Clicking the native **Test passkey** button immediately returned
  `NotAllowedError`, without a system passkey sheet, in the unassociated build.
- This does not isolate user activation from missing domain association.
- No successful PRF evaluation or login has been demonstrated yet.

After association works, use an existing test passkey. Verify both outputs,
cancellation and retry. Then integrate the existing worker custody login as a
separate step; successful output lengths alone do not prove account recovery.

## Association setup checkpoint

The user approved and Apple registered **Tonk Desktop**, `8WVKS2F24C.xyz.tonk`,
with Associated Domains. The separately approved **Tonk Desktop Developer ID**
profile was generated and downloaded to
`~/Downloads/Tonk_Desktop_Developer_ID.provisionprofile`. Its existing certificate
is the Tonk Labs Developer ID Application certificate. The profile is embedded
in the test bundle; it is not checked into Git.

The provisioned build passed host-side `codesign --verify --strict` and entitlement
inspection and launched successfully. Sandboxed `codesign` inspection falsely
reported an invalid entitlement blob; host-side inspection showed all three
expected entitlements. No entitlement workaround was applied.

Focused JavaScript checks passed for the exact PRF inputs, clearing returned
buffers, missing PRF, cancellation, error redaction and origin rejection.

`association-worker.mjs` and `association-wrangler.toml` prepare a temporary
Cloudflare route restricted to the exact AASA path. GET/HEAD, the App ID JSON,
and rejection of other paths/methods passed local checks. The user selected the
temporary route, which was deployed on 2026-10-06:

- Worker: `tonk-desktop-passkey-association`
- Version: `db961c34-d372-4c93-8d06-3145fb7a490d`
- Exact route: `tonk.network/.well-known/apple-app-site-association`
- Public response: HTTP 200, `application/json`, expected App ID.
- Apple's `app-site-association.cdn-apple.com/a/v1/tonk.network` also returned
  the expected JSON. An initial public request during propagation still returned
  the old HTML; the subsequent public check passed.
- The running provisioned probe still returned `NotAllowedError`; a rebuild
  adds DOMException detail to the local test UI (not stdout). PRF bytes and
  credential IDs still never cross into Swift.
- The precise failure is `The document is not focused.` Rebuilt probes that
  make the WKWebView first responder and activate its presenting app still
  encountered this under computer-use clicks. A manual foreground click is
  requested to distinguish automation/background focus from app behavior.
  No successful passkey sheet or PRF evaluation has yet been observed.
- A subsequent manual click reached a generic platform refusal. Fresh
  AuthenticationServicesAgent logs identify the reason: `Application with
  identifier 8WVKS2F24C.xyz.tonk is not associated with domain tonk.network`,
  followed by `Client is not properly entitled for this request. Rejecting.`
  The logged request includes both 19-byte PRF inputs. This is evidence that
  the manual native-button path reached the system authentication service;
  the current blocker is local association validation, not PRF evaluation.
- A scoped `lsregister -f` refreshed this test bundle only. Launch Services
  reports the correct team, application identifier and associated-domain
  entitlement. It does not prove swcd has approved the association. Reading
  `swcutil get -s webcredentials -a 8WVKS2F24C.xyz.tonk -d tonk.network`
  requires administrator authentication; the user has been asked to run that
  read-only diagnostic. `show` does not accept `-d` on this macOS version.
- The user's filtered `swcutil get` returned no record. A separate copy of the
  signed probe was then installed at `~/Applications/Tonk Passkey Test.app`,
  registered and launched. This triggered a new `swcd` CDN download for the
  masked `to….network` domain at 22:04:55. The available downloader logs do not
  establish successful association. The installed copy still hits the focus
  refusal under automated clicks; a manual retry and filtered association query
  are pending. The ordinary Tonk app was not replaced.
- **Resolved:** after installation, the user reran the filtered query and got
  `{ s = webcredentials, a = 8WVKS2F24C.xyz.tonk, d = tonk.network,
  ua = unspecified, sa = approved }`. Fresh inspection of the installed probe
  then showed `Passkey approved. Both 32-byte PRF outputs received. No account
  was changed.` The manual SwiftUI button successfully drove WKWebView's
  system ceremony. Earlier failures above are chronological diagnostic evidence,
  not the current outcome. Do not treat the build-directory bundle's signature
  alone as proof macOS has registered its associated domains.

## Next integration boundary

The ceremony is proven; account login is not. Next, connect the existing
`window.tonkIdentity.usePasskey` / custody-login intent to the desktop sign-in
action, preserving account/profile constraints and handling asynchronous provider
attachment. Verify that it recovers the same account and spaces, then sign-out,
re-login and relaunch. PRF values must remain inside the WebKit/worker boundary.
The shipped desktop bundle also needs the approved Tonk Labs profile and signing
setup; the ordinary app's build script has not yet been changed.

The normal-release alternative is prepared in the isolated checkout
`/private/tmp/tonk-desktop-passkey-association`: AASA JSON under
`rust/tonk-ui/assets/.well-known/`, its Trunk copy rule, and JSON/cache headers.
The normal-release asset change has not been published. After it ships, remove
the temporary worker route using its checked-in Wrangler config. Do not remove
the route before the underlying production asset is available, or new installs
may lose association. The temporary worker has no secrets or service bindings.
