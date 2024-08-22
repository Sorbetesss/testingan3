// Copyright 2024 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use std::{borrow::Cow, env, path::Path, process::Command};

use codec::Decode;
use sc_executor::{WasmExecutionMethod, WasmExecutor};
use sc_executor_common::runtime_blob::RuntimeBlob;
use sp_maybe_compressed_blob::CODE_BLOB_BOMB_LIMIT;
use subxt_codegen::{fetch_metadata::fetch_metadata_from_file_blocking, CodegenError, Metadata};

/// Result type shorthand
pub type WasmMetadataResult<A> = Result<A, CodegenError>;

/// Uses wasm artifact produced by compiling the runtime to generate metadata
pub fn from_wasm_file(wasm_file_path: &Path) -> WasmMetadataResult<Metadata> {
    let wasm_file = fetch_metadata_from_file_blocking(&wasm_file_path)
        .map_err(Into::<CodegenError>::into)
        .and_then(maybe_decompress)?;
    call_and_decode(wasm_file)
}

/// Compiles the runtime to wasm and uses the produced artifact to generate metadata
pub fn from_crate_name(
    runtime_crate_name: &str,
    wasm_file_path: &Path,
    features: Vec<String>,
) -> WasmMetadataResult<Metadata> {
    let cargo = env::var("CARGO").unwrap();

    let mut args = vec!["build", "-p", &runtime_crate_name, "--profile", "release"];
    let features = features.join(",");
    if !features.is_empty() {
        args.push("--features");
        args.push(&features)
    };

    // Deadlocks as we are building already
    Command::new(cargo)
        .env_remove("SKIP_WASM_BUILD")
        .args(&args)
        .status()
        .unwrap();

    from_wasm_file(wasm_file_path)
}

fn call_and_decode(wasm_file: Vec<u8>) -> WasmMetadataResult<Metadata> {
    let mut ext: sp_state_machine::BasicExternalities = Default::default();

    let executor: WasmExecutor<sp_io::SubstrateHostFunctions> = WasmExecutor::builder()
        .with_execution_method(WasmExecutionMethod::default())
        .with_offchain_heap_alloc_strategy(sc_executor::HeapAllocStrategy::Dynamic {
            maximum_pages: Some(64),
        })
        .with_max_runtime_instances(1)
        .with_runtime_cache_size(1)
        .build();

    let runtime_blob =
        RuntimeBlob::new(&wasm_file).map_err(|e| CodegenError::Wasm(e.to_string()))?;
    let metadata_encoded = executor
        .uncached_call(runtime_blob, &mut ext, true, "Metadata_metadata", &[])
        .map_err(|_| CodegenError::Wasm("method \"Metadata_metadata\" doesnt exist".to_owned()))?;

    let metadata = <Vec<u8>>::decode(&mut &metadata_encoded[..]).map_err(CodegenError::Decode)?;

    decode(metadata)
}

fn decode(encoded_metadata: Vec<u8>) -> WasmMetadataResult<Metadata> {
    Metadata::decode(&mut encoded_metadata.as_ref()).map_err(Into::into)
}

fn maybe_decompress(file_contents: Vec<u8>) -> WasmMetadataResult<Vec<u8>> {
    sp_maybe_compressed_blob::decompress(&file_contents.as_ref(), CODE_BLOB_BOMB_LIMIT)
        .map_err(|e| CodegenError::Wasm(e.to_string()))
        .map(Cow::into_owned)
}
