// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

pub mod default;

#[cfg(feature = "native")]
mod native;

#[cfg(feature = "web")]
mod wasm;
#[cfg(feature = "web")]
mod wasm_socket;
