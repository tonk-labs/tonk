// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side of a `fields` resource.

/// The host's side of a `fields` resource: name and value pairs, in
/// order, and whether the guest may change them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    /// Each field, in the order it was added.
    pub entries: Vec<(String, Vec<u8>)>,
    /// Whether the fields belong to a request or a response, which
    /// makes them immutable.
    pub immutable: bool,
}
