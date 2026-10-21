// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side of a `request` resource.

use super::{Body, Fields};

/// The host's side of a `request` resource.
pub struct Request {
    /// The method, such as `GET`, upper case.
    pub method: String,
    /// The scheme, such as `http`, if any.
    pub scheme: Option<String>,
    /// The authority, such as `localhost:8080`, if any.
    pub authority: Option<String>,
    /// The path and query, such as `/api/todos?filter=all`, if any.
    pub path_with_query: Option<String>,
    /// The headers.
    pub headers: Fields,
    /// The body.
    pub body: Body,
}
