// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side of a `response` resource.

use super::{Body, Fields};

/// The host's side of a `response` resource.
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The headers.
    pub headers: Fields,
    /// The body.
    pub body: Body,
}
