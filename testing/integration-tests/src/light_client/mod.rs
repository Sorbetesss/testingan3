// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! # Light Client Initialization and Testing
//!
//! The initialization process of the light client can be slow, especially when
//! it needs to synchronize with a local running node for each individual
//! #[tokio::test] in subxt. To optimize this process, a subset of tests is
//! exposed to ensure the light client remains functional over time. Currently,
//! these tests are placed under an unstable feature flag.
//!
//! Ideally, we would place the light client initialization in a shared static
//! using `OnceCell`. However, during the initialization, tokio::spawn is used
//! to multiplex between subxt requests and node responses. The #[tokio::test]
//! macro internally creates a new Runtime for each individual test. This means
//! that only the first test, which spawns the substrate binary and synchronizes
//! the light client, would have access to the background task. The cleanup process
//! would destroy the spawned background task, preventing subsequent tests from
//! accessing it.
//!
//! To address this issue, we can consider creating a slim proc-macro that
//! transforms the #[tokio::test] into a plain #[test] and runs all the tests
//! on a shared tokio runtime. This approach would allow multiple tests to share
//! the same background task, ensuring consistent access to the light client.
//!
//! For more context see: https://github.com/tokio-rs/tokio/issues/2374.
//!

use crate::{test_context, utils::node_runtime};
use codec::{Compact, Encode};
use futures::StreamExt;
use subxt::client::{LightClient, OfflineClientT, OnlineClientT};
use subxt::config::SubstrateConfig;
use subxt::rpc::types::FollowEvent;
use subxt::utils::AccountId32;
use subxt_metadata::Metadata;
use subxt_signer::sr25519::dev;

// We don't use these dependencies.
use assert_matches as _;
use frame_metadata as _;
use hex as _;
use regex as _;
use scale_info as _;
use sp_core as _;
use subxt_codegen as _;
use syn as _;

// Check that we can subscribe to non-finalized blocks.
async fn non_finalized_headers_subscription(
    api: &LightClient<SubstrateConfig>,
) -> Result<(), subxt::Error> {
    let mut sub = api.blocks().subscribe_best().await?;
    let header = sub.next().await.unwrap()?;
    let block_hash = header.hash();
    let current_block_hash = api.rpc().block_hash(None).await?.unwrap();

    assert_eq!(block_hash, current_block_hash);

    let _block = sub.next().await.unwrap()?;
    let _block = sub.next().await.unwrap()?;
    let _block = sub.next().await.unwrap()?;

    Ok(())
}

// Check that we can subscribe to finalized blocks.
async fn finalized_headers_subscription(
    api: &LightClient<SubstrateConfig>,
) -> Result<(), subxt::Error> {
    let mut sub = api.blocks().subscribe_finalized().await?;
    let header = sub.next().await.unwrap()?;
    let finalized_hash = api.rpc().finalized_head().await?;

    assert_eq!(header.hash(), finalized_hash);

    let _block = sub.next().await.unwrap()?;
    let _block = sub.next().await.unwrap()?;
    let _block = sub.next().await.unwrap()?;

    Ok(())
}

// Check that we can subscribe to non-finalized blocks.
async fn runtime_api_call(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let mut sub = api.blocks().subscribe_best().await?;

    let block = sub.next().await.unwrap()?;
    let rt = block.runtime_api().await?;

    // get metadata via state_call.
    let (_, meta1) = rt
        .call_raw::<(Compact<u32>, Metadata)>("Metadata_metadata", None)
        .await?;

    // get metadata via `state_getMetadata`.
    let meta2 = api.rpc().metadata_legacy(None).await?;

    // They should be the same.
    assert_eq!(meta1.encode(), meta2.encode());

    Ok(())
}

// Fetch the account nonce of Alice using the runtime API before
// and after submitting an extrinsic.
async fn runtime_api_account_nonce(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let alice = dev::alice();
    let alice_account_id: AccountId32 = alice.public_key().into();

    // Check Alice nonce is starting from 0.
    let runtime_api_call = node_runtime::apis()
        .account_nonce_api()
        .account_nonce(alice_account_id.clone());
    let nonce = api
        .runtime_api()
        .at_latest()
        .await?
        .call(runtime_api_call)
        .await?;
    assert_eq!(nonce, 0);

    // Do some transaction to bump the Alice nonce to 1:
    let remark_tx = node_runtime::tx().system().remark(vec![1, 2, 3, 4, 5]);
    api.tx()
        .sign_and_submit_then_watch_default(&remark_tx, &alice)
        .await?
        .wait_for_finalized_success()
        .await?;

    let runtime_api_call = node_runtime::apis()
        .account_nonce_api()
        .account_nonce(alice_account_id);
    let nonce = api
        .runtime_api()
        .at_latest()
        .await?
        .call(runtime_api_call)
        .await?;
    assert_eq!(nonce, 1);

    Ok(())
}

// Lookup for the `Timestamp::now` plain storage entry.
async fn storage_plain_lookup(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let addr = node_runtime::storage().timestamp().now();
    let entry = api
        .storage()
        .at_latest()
        .await?
        .fetch_or_default(&addr)
        .await?;
    assert!(entry > 0);

    Ok(())
}

// Subscribe to produced blocks using the `ChainHead` spec V2 and fetch the header of
// just a few reported blocks.
async fn follow_chain_head(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let mut blocks = api.rpc().chainhead_unstable_follow(false).await?;
    let sub_id = blocks
        .subscription_id()
        .expect("RPC provides a valid subscription id; qed")
        .to_owned();

    let event = blocks.next().await.unwrap()?;
    if let FollowEvent::BestBlockChanged(best_block) = event {
        let hash = best_block.best_block_hash;
        let _header = api
            .rpc()
            .chainhead_unstable_header(sub_id.clone(), hash)
            .await?
            .unwrap();
    }

    let event = blocks.next().await.unwrap()?;
    if let FollowEvent::BestBlockChanged(best_block) = event {
        let hash = best_block.best_block_hash;
        let _header = api
            .rpc()
            .chainhead_unstable_header(sub_id.clone(), hash)
            .await?
            .unwrap();
    }
    Ok(())
}

// Make a dynamic constant query for `System::BlockLenght`.
async fn dynamic_constant_query(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let constant_query = subxt::dynamic::constant("System", "BlockLength");
    let _value = api.constants().at(&constant_query)?;

    Ok(())
}

// Fetch a few all events from the latest block and decode them dynamically.
async fn dynamic_events(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let events = api.events().at_latest().await?;

    for event in events.iter() {
        let _event = event?;
    }

    Ok(())
}

// Make a few raw RPC calls to the chain.
async fn various_rpc_calls(api: &LightClient<SubstrateConfig>) -> Result<(), subxt::Error> {
    let _system_chain = api.rpc().system_chain().await?;
    let _system_name = api.rpc().system_name().await?;
    let _finalized_hash = api.rpc().finalized_head().await?;

    Ok(())
}

#[tokio::test]
async fn light_client_testing() -> Result<(), subxt::Error> {
    tracing_subscriber::fmt::init();

    let ctx = test_context().await;
    let api = ctx.client();

    non_finalized_headers_subscription(&api).await?;
    finalized_headers_subscription(&api).await?;
    runtime_api_call(&api).await?;
    runtime_api_account_nonce(&api).await?;
    storage_plain_lookup(&api).await?;
    follow_chain_head(&api).await?;
    dynamic_constant_query(&api).await?;
    dynamic_events(&api).await?;
    various_rpc_calls(&api).await?;

    Ok(())
}
