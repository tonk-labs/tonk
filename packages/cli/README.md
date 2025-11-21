# Tonk CLI

A command-line tool for Tonk authentication using WebAuthn PRF extension and capability delegation.

## Overview

The Tonk CLI implements a secure authentication flow that combines:
- **WebAuthn with PRF Extension**: Derives cryptographic keys from biometric authentication
- **Operator Model**: CLI generates a persistent operator keypair stored in OS keyring
- **Capability Delegation**: Authority (derived from WebAuthn) delegates capabilities to the operator
- **Simplified UCAN**: Uses a lightweight delegation format that can be upgraded to full UCAN later

## Quick Start

```bash
# Build the CLI
cargo build --release

# Run login with local auth page
./target/release/tonk login

# Run login with custom auth URL and duration
./target/release/tonk login --via https://auth.tonk.xyz --duration 7d
```

## Architecture

### Authentication Flow

```
┌─────────────────────────────────────────────────────────────┐
│ CLI Process                                                  │
│                                                              │
│ 1. Generate/load Operator keypair from OS keyring           │
│    - Stored as: "tonk-cli/operator-keypair"                 │
│    - Persistent across sessions                             │
│                                                              │
│ 2. Start callback server (localhost:8089)                   │
│                                                              │
│ 3a. If --via provided:                                      │
│     - Open: {via}?as={operator_did}&cmd=/&sub=null&         │
│               callback=http://localhost:8089&duration=secs  │
│                                                              │
│ 3b. If --via NOT provided:                                  │
│     - Start auth server (localhost:8088) serving auth.html  │
│     - Open: http://localhost:8088?as={operator_did}&...     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Browser (auth.html)                                          │
│                                                              │
│ 1. Parse query parameters (as, cmd, sub, callback, duration)│
│                                                              │
│ 2. Perform WebAuthn authentication with PRF extension       │
│    - Salt: "tonk-authority-v1"                              │
│    - Returns 32-byte PRF output                             │
│                                                              │
│ 3. Derive Authority Ed25519 keypair using HKDF              │
│    - Input: PRF output                                      │
│    - Salt: "tonk-authority-v1"                              │
│    - Info: "ed25519"                                        │
│                                                              │
│ 4. Generate Authority did:key                               │
│    - Format: did:key:z{base58(multicodec + pubkey)}         │
│    - Multicodec: 0xed01 (Ed25519)                           │
│                                                              │
│ 5. Create delegation payload                                │
│    {                                                         │
│      iss: authority_did,                                    │
│      aud: operator_did,                                     │
│      cmd: "/",                                              │
│      sub: null,                                             │
│      exp: now + duration,                                   │
│      pol: []                                                │
│    }                                                         │
│                                                              │
│ 6. Sign payload with Authority private key                  │
│                                                              │
│ 7. POST form to callback URL                                │
│    - Field: "authorize" = JSON { payload, signature }       │
│    - OR: "deny" if user denies                              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Callback Server (localhost:8089)                            │
│                                                              │
│ 1. Receive POST request                                     │
│                                                              │
│ 2. Parse delegation from "authorize" field                  │
│                                                              │
│ 3. Validate delegation                                      │
│    - Check audience matches operator DID                    │
│    - Check not expired                                      │
│                                                              │
│ 4. Save delegation to ~/.tonk/delegation.json               │
│                                                              │
│ 5. Return success HTML page                                 │
│                                                              │
│ 6. Shutdown servers                                         │
└─────────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. Operator Keypair (`crypto.rs`, `keystore.rs`)
- **Purpose**: Represents the CLI program itself
- **Storage**: OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- **Lifetime**: Persistent across sessions
- **DID Format**: `did:key:z{base58btc-encoded-ed25519-pubkey}`

#### 2. Authority (`auth.html`)
- **Derivation**: WebAuthn PRF extension → HKDF → Ed25519 keypair
- **Purpose**: User's cryptographic identity derived from biometric auth
- **Storage**: Non-extractable (exists only during auth session in browser)
- **PRF Salt**: `"tonk-authority-v1"`

#### 3. Delegation (`delegation.rs`)
- **Format**: Simplified UCAN structure
  ```rust
  {
    payload: {
      iss: "did:key:z...",  // Authority DID
      aud: "did:key:z...",  // Operator DID
      cmd: "/",              // Command/capability
      sub: null,             // Subject
      exp: 1234567890,       // Unix timestamp
      pol: []                // Policy (empty for now)
    },
    signature: "base64..."   // Ed25519 signature
  }
  ```
- **Storage**: `~/.tonk/delegation.json`
- **Validation**: Checked for expiration and audience match

#### 4. Two-Server Architecture (`login.rs`)
- **Auth Server** (optional, localhost:8088): Serves `auth.html`
- **Callback Server** (required, localhost:8089): Receives delegation via form POST
- **Port Discovery**: Automatically finds available ports

## CLI Commands

### `tonk login`

Authenticate and obtain delegated capabilities.

**Options:**
- `--via <URL>`: Optional authentication URL (default: local auth server)
- `--duration <DURATION>`: Session duration (default: "30d")

**Duration Format:**
- `s` - seconds
- `m` - minutes
- `h` - hours
- `d` - days

**Examples:**
```bash
# Local auth with default 30-day session
tonk login

# Local auth with custom duration
tonk login --duration 7d

# Remote auth URL with 1-hour session
tonk login --via https://auth.tonk.xyz --duration 1h
```

### `tonk status`

Display operator identity and access to subjects (spaces/DIDs).

**Options:**
- `--verbose`, `-v`: Show detailed information (full DIDs, file paths, metadata)

**What it shows:**
- **Operator DID** - Your CLI's identity (shortened by default)
- **Subjects** - What DIDs/spaces you have access to
- **Commands** - Capabilities for each subject (e.g., `/` for full access, `/store/*` for specific commands)
- **Expiration** - Human-readable time remaining (e.g., "5d 3h", "2h 15m")
- **Delegation path** - Whether access is `[direct]` or `[via authority]`

**Powerline Delegations:**
When a delegation has `sub: null`, it's a powerline delegation that:
1. Grants access to the **issuer's DID itself** (the authority)
2. Grants access to **everything the authority has access to** (recursive)

The status command resolves this chain and shows all accessible subjects in a flat tree.

**Examples:**

```bash
# Compact view (default)
tonk status

# Detailed view with full DIDs, file paths, and metadata
tonk status --verbose
```

**Normal Output:**
```
📊 Status

🤖 Operator: did:key:z6Mkpp5...biCPbAv9

📜 Access:

  did:key:z6MkAuth...thority1
  └─  / (expires: 29d 23h) [direct]

  did:key:z6MkSpace...pace123
  ├─  /store/* (expires: 5d 3h) [direct]
  └─  /share/* (expires: 3d 2h) [via did:key:z6MkAut...rity2]
```

**Verbose Output (`--verbose`):**
```
📊 Status

🤖 Operator: did:key:z6Mkpp5uVgJMybDRhZGwfB8uXJ1zmDQn83ber5JVbiCPbAv9

📜 Access:

  did:key:z6MkpE4x2Kqnt6iZ9WPtUqm7Dj2JthPetz9bGZjj3MRTF1Hh
  └─  / (expires: 29d 23h) [direct from did:key:z6MkpE4x2Kqnt6iZ9WPtUqm7Dj2JthPetz9bGZjj3MRTF1Hh]
        File: ~/.tonk/access/did:key:z6Mkpp5.../did:key:z6MkpE4.../1766268778-01cb7e35.json
        Source: http://localhost:8088 (Local)

  did:key:z6MkSpace1234567890abcdefghijklmnopqrstuvwxyz
  ├─  /store/* (expires: 5d 3h) [direct from did:key:z6MkSpace1234...]
  │     File: ~/.tonk/access/did:key:z6Mkpp5.../did:key:z6MkSpace.../1766123456-a1b2c3d4.json
  │     Source: https://auth.example.com (Remote)
  └─  /share/* (expires: 3d 2h) [via did:key:z6MkAuthority... (1 hop)]
        File: ~/.tonk/access/did:key:z6Mkpp5.../did:key:z6MkAuth.../1766789012-e5f6g7h8.json
        Source: http://localhost:8088 (Local)
```

**Tree Structure:**
- Each subject is a top-level entry
- Commands are shown as tree branches under each subject
- `[direct]` means you have a direct delegation to that subject
- `[via authority]` means you have access through a powerline delegation chain
- Multiple paths to the same capability are shown as multiple branches
- Expired delegations are automatically filtered out

## Query Parameters

When opening the auth page (local or remote), the following query parameters are passed:

| Parameter  | Description                          | Example                 |
|------------|--------------------------------------|-------------------------|
| `as`       | Operator DID (audience)              | `did:key:z6Mk...`      |
| `cmd`      | Command/capability being requested   | `/` (powerline access) |
| `sub`      | Subject (resource identifier)        | `null`                 |
| `callback` | Callback URL for form POST           | `http://localhost:8089`|
| `duration` | Session duration in seconds          | `2592000` (30 days)    |

## Callback Protocol

The auth page must POST to the `callback` URL with one of:

**Authorization:**
```html
<form method="POST" action="{callback}">
  <input name="authorize" value='{"payload":{...},"signature":"..."}'>
</form>
```

**Denial:**
```html
<form method="POST" action="{callback}">
  <input name="deny" value="User denied authorization">
</form>
```

## WebAuthn Implementation

The `auth.html` page implements WebAuthn with PRF extension:

### Browser Compatibility

- ✅ **Chrome/Edge**: Full support with platform authenticators
- ✅ **Safari (macOS 17+)**: PRF extension supported
- ❌ **Firefox**: PRF extension not yet supported
- ⚠️ **Safari (iOS)**: Limited PRF support

### Key Derivation

```javascript
// 1. Get PRF output from WebAuthn
const credential = await navigator.credentials.create({
  extensions: {
    prf: {
      eval: {
        first: new TextEncoder().encode("tonk-authority-v1")
      }
    }
  }
});
const prfOutput = credential.getClientExtensionResults().prf.results.first;

// 2. Derive Ed25519 seed using HKDF
const authoritySeed = await crypto.subtle.deriveBits({
  name: "HKDF",
  hash: "SHA-256",
  salt: new TextEncoder().encode("tonk-authority-v1"),
  info: new TextEncoder().encode("ed25519")
}, prfKey, 256);

// 3. Generate Ed25519 keypair
const authorityPrivateKey = new Uint8Array(authoritySeed);
const authorityPublicKey = await ed25519.getPublicKey(authorityPrivateKey);

// 4. Sign delegation
const signature = await ed25519.sign(payloadBytes, authorityPrivateKey);
```

## File Structure

```
packages/cli/
├── Cargo.toml                # Rust dependencies
├── README.md                 # This file
├── auth.html                 # WebAuthn authentication page
└── src/
    ├── main.rs               # CLI entry point
    ├── crypto.rs             # Ed25519 & DID:key implementation
    ├── keystore.rs           # OS keyring integration
    ├── delegation.rs         # Delegation format, storage & verification
    ├── login.rs              # Two-server authentication flow
    └── status.rs             # Status command - display delegations
```

## Storage Structure

Delegations are stored in a hierarchical structure:
```
~/.tonk/access/{aud}/{sub or iss}/{exp}-{hash}.json
```

- **Regular delegation** (with specific subject): `{aud}/{sub}/{exp}-{hash}.json`
- **Powerline delegation** (sub is null): `{aud}/{iss}/{exp}-{hash}.json`

When `sub` is `null`, the issuer DID is used as the directory name instead, making it easy to identify powerline delegations.

Example:
```
~/.tonk/
└── access/
    └── did:key:z6MkOperator.../           # Operator DID (audience)
        ├── did:key:z6MkAuthority.../      # Powerline delegation (issuer as directory)
        │   └── 1739612400-a3f5e2d8.json  # sub: null (powerline)
        └── did:key:z6MkSpace.../          # Space-specific delegation (subject as directory)
            └── 1739612400-b4c6f3e9.json  # sub: did:key:z6MkSpace...
```

## Dependencies

### Rust
- `clap` - CLI argument parsing
- `ed25519-dalek` - Ed25519 cryptography
- `keyring` - OS keyring access
- `axum` + `tokio` - HTTP servers
- `bs58` - Base58 encoding
- `serde` + `serde_json` - Serialization

### Browser (auth.html)
- `@noble/ed25519` (via esm.sh) - Ed25519 signing
- Web Crypto API - HKDF key derivation
- WebAuthn API - Biometric authentication with PRF

## Security Considerations

1. **Operator Key Storage**: Stored in OS keyring with platform-specific encryption
2. **Authority Derivation**: Never stored; derived fresh each authentication from PRF
3. **PRF Origin Binding**: Keys are bound to the browser origin
4. **Delegation Transport**: Only signed delegations transmitted (no private keys)
5. **Session Expiration**: Enforced via `exp` field in delegation payload

## Future Enhancements

### UCAN Migration
The current simplified delegation format SHOULD be upgraded to full UCAN

## Development

```bash
# Build
cargo build

# Run
cargo run -- login --duration 7d

# Test
cargo test

# Build release
cargo build --release
```

## References

- [Share Sprint Spec](../../notes/Share%20Sprint.md)
- [WebAuthn PRF Extension](https://w3c.github.io/webauthn/#prf-extension)
- [UCAN Specification](https://github.com/ucan-wg/spec)
- [DID:key Method](https://w3c-ccg.github.io/did-method-key/)
- [HKDF (RFC 5869)](https://tools.ietf.org/html/rfc5869)
