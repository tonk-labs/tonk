// Copyright 2026 The Dialog DB Project
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A body.

use wcmp::Val;

/// A body: bytes the host holds, or a stream the guest writes.
pub enum Body {
    /// No body.
    Empty,
    /// The whole body.
    Bytes(Vec<u8>),
    /// A `stream<u8>` the guest writes, read once the host needs it.
    Stream(Val),
}
