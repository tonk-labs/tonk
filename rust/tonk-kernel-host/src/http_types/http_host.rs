// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The store data of a component that imports `wasi:http/types`.

use super::HttpTable;

/// The store data of a component that imports [`TYPES`]: it holds the
/// host's side of every HTTP resource.
///
/// [`TYPES`]: super::TYPES
pub trait HttpHost: 'static {
    /// The table of HTTP resources.
    fn http(&mut self) -> &mut HttpTable;
}
