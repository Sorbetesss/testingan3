// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

#![deny(unused_crate_dependencies)]

#[cfg(test)]
mod codegen;
#[cfg(test)]
mod utils;

#[cfg(test)]
mod blocks;
#[cfg(test)]
mod client;
#[cfg(test)]
mod frame;
#[cfg(test)]
mod metadata;
#[cfg(test)]
mod runtime_api;
#[cfg(test)]
mod storage;

#[cfg(test)]
use test_runtime::node_runtime;
#[cfg(test)]
use utils::*;

// The sp_runtime dependency is used for non light-client tests.
#[cfg(test)]
use sp_runtime as _;

// We don't use this dependency, but it's here so that we
// can enable logging easily if need be. Add this to a test
// to enable tracing for it:
//
// tracing_subscriber::fmt::init();
#[cfg(test)]
use tracing_subscriber as _;

// The following are used by `contracts` tests disabled for lightclient.
#[cfg(test)]
use tracing as _;
#[cfg(test)]
use wabt as _;
