# Tonk CLI

Command-line tool for Tonk authentication and space collaboration.

## Overview

The Tonk CLI provides:

- **Session Management**: Login with authorities and manage active sessions
- **Space Collaboration**: Create spaces, invite collaborators, and join shared spaces
- **UCAN Delegations**: Uses UCAN delegations for access sharing and capability management
- **Operator Model**: CLI generates a persistent operator keypair stored in OS keyring
- **Email-based Invitations**: Invite collaborators using email addresses with derived membership
  keys

## Quick Start

```bash
# Build the CLI
cargo build --release

# Login to create a session
tonk login

# View your sessions and spaces
tonk session
tonk space

# Create a space
tonk space create myspace

# Invite collaborator to a space
tonk space invite alice@example.com --space myspace

# Join a space using an invite file
tonk space join --invite path/to/invite.cbor
```

## Architecture

### Key Concepts

#### 1. Operator (CLI Identity)

- **Purpose**: Represents the CLI program itself
- **Storage**: OS keyring or `TONK_OPERATOR_KEY` environment variable (base58btc-encoded)
- **Lifetime**: Persistent across sessions
- **Identity**: Identified by `did:key`

#### 2. Authority (User Identity)

- **Purpose**: User's cryptographic identity, grants capabilities to operator
- **Creation**: Obtained through authentication (e.g., WebAuthn with PRF)
- **Relationship**: Issues powerline delegations (`*` subject) to operator
- **Identity**: Identified by `did:key`

#### 3. Space (Collaboration Unit)

- **Purpose**: Shared resource with multiple owners/collaborators
- **Identity**: Identified by `did:key`
- **Access**: Controlled via UCAN delegation chains

#### 4. Session

- **Definition**: An active authority context
- **Active Marker**: `~/.tonk/operator/{operator-did}/session/@active`
- **Per-Session State**: Each session can have its own active space

### UCAN Delegation Chains

Access is represented as delegation chains from resources to operators:

```
Space DID → Authority DID → Operator DID
```

**Powerline Delegations** (`subject: *`):

```
Authority DID → Operator DID  (operator can act as authority for anything)
```

**Space Access**:

```
Space DID → Owner DID → Operator DID
```

**Invitation Flow**:

```
Space DID → Authority DID → Operator DID → Membership DID (did:mailto)
```

## Storage Structure

```
~/.tonk/
├── operator/
│   └── {operator-did}/
│       └── session/
│           ├── @active                    # Active session (plain text: authority DID)
│           └── {authority-did}/
│               └── space/
│                   ├── @active            # Active space (plain text: space DID)
│                   ├── {space-did}/       # Space directory
│                   └── ...
├── access/
│   └── {audience-did}/
│       ├── {issuer-did}/                  # Powerline delegations (subject=*)
│       │   └── {exp}-{hash}.cbor
│       └── {subject-did}/                 # Specific subject delegations
│           └── {exp}-{hash}.cbor
└── meta/
    ├── {authority-did}/
    │   └── session.json                   # Session metadata (name, via, created_at)
    └── {space-did}/
        └── space.json                     # Space metadata (name, owners, created_at)
```

### File Formats

- **`@active` files**: Plain text containing a DID (Git HEAD-style)
- **Delegation files**: DAG-CBOR encoded UCAN delegations
- **Metadata files**: JSON objects with display names and timestamps
- **Invite files**: DAG-CBOR with delegation chain and invite code

## Commands

### `tonk login`

Authenticate and obtain delegated capabilities.

```bash
# Local auth with default settings
tonk login

# Remote auth URL
tonk login --via https://auth.tonk.xyz
```

**Options:**

- `--via <URL>`: Optional authentication URL (default: local auth server)

### `tonk session`

Manage sessions (authority contexts).

```bash
# List all sessions
tonk session

# List with detailed delegation chains
tonk session -v

# Show current session DID
tonk session current

# Switch to different session
tonk session set did:key:z6Mkp8ik8emh...
```

**Output shows:**

- Authority and operator DIDs
- Available spaces with access capabilities
- Delegation chains (in verbose mode)

### `tonk space`

Manage spaces (collaboration units).

```bash
# List spaces for active session
tonk space

# Show current active space
tonk space current

# Switch to different space (by name or DID)
tonk space set myspace
tonk space set did:key:z6Mkg7rGV...

# Create a new space
tonk space create myspace

# Create with specific owners
tonk space create myspace --owners did:key:z6Mk... did:key:z6Mk...

# Invite collaborator
tonk space invite alice@example.com

# Invite to specific space
tonk space invite alice@example.com --space myspace

# Join space with invitation
tonk space join --invite /path/to/invite.cbor
```

**Space Creation:**

- Always includes active authority as owner
- Can specify additional owners interactively or via `--owners`
- Creates delegation chain: Space → each Owner

**Invitations:**

- Uses `did:mailto:email@example.com` as placeholder
- Creates invitation delegation: Operator → did:mailto
- Generates invite code (last 5 chars of hash, base58btc)
- Outputs `.cbor` invite file

**Joining:**

- Derives membership keypair from HKDF(invitation_hash + invite_code)
- Replaces `did:mailto` with derived membership DID
- Saves complete delegation chain

### `tonk inspect`

Debug and inspect delegations.

```bash
# Inspect CBOR delegation file
tonk inspect delegation path/to/delegation.cbor

# Inspect base64-encoded CBOR delegation
tonk inspect delegation <base64-string>

# Inspect invite file
tonk inspect invite path/to/invite.cbor
```

Shows:

- Issuer, audience, subject
- Command/capabilities
- Expiration and validity
- Serialization roundtrip verification

## Authentication Flow

```
┌─────────────────────────────────────────────────────────────┐
│ CLI Process                                                  │
│                                                              │
│ 1. Get/create operator keypair from OS keyring              │
│ 2. Start callback server (localhost:8089)                   │
│ 3. Open auth page with parameters:                          │
│    - as: operator DID                                        │
│    - cmd: /                                                  │
│    - sub: *  (powerline access)                             │
│    - callback: http://localhost:8089                        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Browser (auth.html)                                          │
│                                                              │
│ 1. WebAuthn authentication with PRF extension               │
│ 2. Derive Authority Ed25519 keypair from PRF output         │
│ 3. Create UCAN delegation: Authority → Operator             │
│ 4. Serialize to DAG-CBOR, encode as base64                  │
│ 5. POST to callback: { authorize: "<base64-cbor>" }         │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Callback Server                                             │
│                                                              │
│ 1. Decode base64, parse DAG-CBOR UCAN                       │
│ 2. Validate delegation (audience, expiration)               │
│ 3. Save to ~/.tonk/access/{operator}/{authority}/...cbor    │
│ 4. Save metadata to ~/.tonk/meta/{authority}/session.json   │
│ 5. Set active session: ~/.tonk/operator/.../session/@active │
│ 6. Return success and shutdown                              │
└─────────────────────────────────────────────────────────────┘
```

## Operator Keys

The operator key identifies your CLI instance. By default, it's automatically generated and stored
in your OS keyring on first use.

### Generating Keys Manually

You can generate a new operator key for use with the `TONK_OPERATOR_KEY` environment variable:

```bash
tonk operator generate
```

This outputs:

```
Generated new operator key:

🫆 did:key:z6MkpFN1nbyyPymiUXWQuQf6gn4mz7zTar9fJRe6kMZQaFJz
🔑 F6pFiqh1Ykk44JFrCtifdNXx1A3bGWzo2XKxGRBWXRCC

To use this operator:
  export TONK_OPERATOR_KEY=F6pFiqh1Ykk44JFrCtifdNXx1A3bGWzo2XKxGRBWXRCC
```

### Use Cases

- **Multiple Profiles**: Use different keys for different contexts (work, personal, testing)
- **CI/CD**: Set `TONK_OPERATOR_KEY` in CI environments
- **Shared Keys**: Share a key across team members or machines

### Environment Variable

```bash
# Use a specific operator key
export TONK_OPERATOR_KEY=F6pFiqh1Ykk44JFrCtifdNXx1A3bGWzo2XKxGRBWXRCC
tonk session

# One-time use
TONK_OPERATOR_KEY=<key> tonk space
```

When `TONK_OPERATOR_KEY` is set, the CLI:

- Uses that key instead of the OS keyring
- Stores data under `~/.tonk/operator/{operator-did}/`
- Each key has its own isolated data directory

## DID Methods

### `did:key` (Cryptographic Identity)

Used for operators, authorities, and spaces. Derived from Ed25519 public keys.

**Format**: `did:key:z{base58btc(multicodec + pubkey)}`

- Multicodec: `0xed01` (Ed25519 public key)
- Example: `did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK`

### `did:mailto` (Email Placeholder)

Used for email-based invitations before membership key derivation.

**Format**: `did:mailto:{email}`

- Example: `did:mailto:alice@example.com`
- Not cryptographic, serves as placeholder only
- Replaced with derived membership DID during join

## Security Considerations

1. **Operator Key**: Stored encrypted in OS keyring (Keychain, Credential Manager, Secret Service)
2. **Delegation Chains**: All access is capability-based via signed UCAN chains
3. **Invite Codes**: Membership keys derived from HKDF(invitation_hash + invite_code)
4. **Expiration**: All delegations include expiration timestamps
5. **Immutable Storage**: Delegations stored with hash-based filenames, preserving exact bytes

## Development

```bash
# Build
cargo build

# Run with specific operator key
TONK_OPERATOR_KEY=<base58-key> cargo run --bin tonk -- session

# Test
cargo test

# Build release
cargo build --release
```

## References

- [UCAN Specification](https://github.com/ucan-wg/spec)
- [DID:key Method](https://w3c-ccg.github.io/did-method-key/)
- [WebAuthn PRF Extension](https://w3c.github.io/webauthn/#prf-extension)
- [DAG-CBOR Specification](https://ipld.io/specs/codecs/dag-cbor/)
- [HKDF (RFC 5869)](https://tools.ietf.org/html/rfc5869)
