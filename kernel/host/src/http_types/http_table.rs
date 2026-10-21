// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side of every HTTP resource of one store.

use std::collections::HashMap;

use wcmp::Error;

use super::{Fields, Request, Response, Types};

/// The host's side of every HTTP resource of one store, by
/// representation.
#[derive(Default)]
pub struct HttpTable {
    next: u32,
    types: Option<Types>,
    /// Each `fields` resource, by representation.
    pub fields: HashMap<u32, Fields>,
    /// Each `request` resource, by representation.
    pub requests: HashMap<u32, Request>,
    /// Each `response` resource, by representation.
    pub responses: HashMap<u32, Response>,
}

impl HttpTable {
    /// A fresh representation.
    pub fn rep(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// Keep `request` and answer its representation, for a handle the
    /// host makes with [`request_handle`].
    ///
    /// [`request_handle`]: super::request_handle
    pub fn insert_request(&mut self, request: Request) -> u32 {
        let rep = self.rep();
        self.requests.insert(rep, request);
        rep
    }

    /// Take the request of `rep` out of the table.
    pub fn take_request(&mut self, rep: u32) -> Option<Request> {
        self.requests.remove(&rep)
    }

    /// Keep `response` and answer its representation.
    pub fn insert_response(&mut self, response: Response) -> u32 {
        let rep = self.rep();
        self.responses.insert(rep, response);
        rep
    }

    /// Take the response of `rep` out of the table.
    pub fn take_response(&mut self, rep: u32) -> Option<Response> {
        self.responses.remove(&rep)
    }

    /// Take the resource types [`define`] registered.
    ///
    /// [`define`]: super::define
    pub fn set_types(&mut self, types: Option<Types>) {
        self.types = types;
    }

    /// The resource types, once the linker registered them.
    pub fn types(&self) -> Result<Types, Error> {
        self.types.ok_or_else(|| Error::Internal {
            message: "the store's HTTP table has no resource types".to_string(),
        })
    }
}
