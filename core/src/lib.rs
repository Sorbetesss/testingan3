// Copyright 2019-2024 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! # Subxt-core
//!
//! `#[no_std]` compatible core crate for subxt.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

pub mod client;
pub mod config;
pub mod constants;
pub mod custom_values;
pub mod dynamic;
pub mod error;
pub mod events;
pub mod metadata;
pub mod runtime_api;
pub mod signer;
pub mod storage;
pub mod tx;
pub mod utils;

pub use config::Config;
pub use error::Error;
pub use signer::Signer;
pub use metadata::Metadata;

#[macro_use]
mod macros;
