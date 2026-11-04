// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The resource types of `wasi:http/types`.

/// The resource types [`define`] registered, which a store's
/// [`HttpTable`] needs before the host makes a handle. Each is a
/// dynamic host resource type, `ResourceType::host_dynamic(n)`, and
/// is kept here as its `n`, which a `ResourceDynamic` names it by.
///
/// [`define`]: super::define
/// [`HttpTable`]: super::HttpTable
#[derive(Debug, Clone, Copy)]
pub struct Types {
    /// The dynamic resource type of `fields`.
    pub fields: u32,
    /// The dynamic resource type of `request`.
    pub request: u32,
    /// The dynamic resource type of `response`.
    pub response: u32,
}

impl Types {
    /// The dynamic resource type of `request`, for a handle the host
    /// makes with `ResourceDynamic::new_own` before a driver runs.
    pub fn request(&self) -> u32 {
        self.request
    }
}
