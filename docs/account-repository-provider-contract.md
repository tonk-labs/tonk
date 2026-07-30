# Account repository provider contract (V1)

A remote compatible with Tonk's root-owned account repository must:

- distinguish a missing branch revision cell from authorization failures,
  outages, malformed responses, and other errors;
- implement conditional first publication (`If-None-Match: *` semantics), so
  exactly one empty genesis revision wins;
- preserve compare-and-set and non-fast-forward behavior for later updates;
- authorize a repository whose immutable subject is the account root through
  the existing root-to-device-to-operator delegation chain, without requiring
  root key material on the device; and
- enforce revocation for every device hop presented in that chain, or clearly
  document that it provides weaker security.

Tonk's provider screens device delegations against the account revocation
registry and fails closed when no valid cached verdict is available.

V1 repository contents are not end-to-end encrypted. The storage provider may
be able to inspect account facts. Only non-secret account metadata belongs in
this repository unless a later repository layer adds encryption.
