//! Example to utilize the `reconnecting rpc client` in subxt
//! which hidden behind behind `--feature unstable-reconnecting-rpc-client`
//!
//! To utilize full logs from the RPC client use:
//! `RUST_LOG="jsonrpsee=trace,reconnecting_jsonrpsee_ws_client=trace"`

#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use subxt::backend::legacy::LegacyBackend;
use subxt::backend::rpc::reconnecting_rpc_client::{Client, RetryPolicy};
use subxt::backend::rpc::RpcClient;
use subxt::config::Header;
use subxt::error::{Error, RpcError};
use subxt::{OnlineClient, PolkadotConfig};

// Generate an interface that we can use from the node's metadata.
#[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_small.scale")]
pub mod polkadot {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create a new client with with a reconnecting RPC client.
    let rpc = Arc::new(
        Client::builder()
            // Reconnect with exponential backoff
            .retry_policy_for_reconnect(
                RetryPolicy::exponential(Duration::from_millis(100))
                    .with_max_delay(Duration::from_secs(10))
                    .with_max_retries(usize::MAX),
            )
            .build("ws://localhost:9944".to_string())
            .await?,
    );

    let backend: LegacyBackend<PolkadotConfig> =
        LegacyBackend::builder().build(RpcClient::new(rpc.clone()));

    let api: OnlineClient<PolkadotConfig> = OnlineClient::from_backend(Arc::new(backend)).await?;

    // The retry-able rpc backend will re-run this until it's succesful.
    // It's also possible to run custom retry_logic withot the retry-backend
    //
    // Then you can use `subxt::backend::utils::retry` or `subxt::backend::utils::retry_with_strategy`
    // to retry rpc calls or write your own retry logic.
    let mut blocks_sub = api.backend().stream_finalized_block_headers().await?;

    // For each block, print a bunch of information about it:
    while let Some(block) = blocks_sub.next().await {
        let header = match block {
            Ok((header, _)) => header,
            Err(e) => {
                return Err(e.into());
            }
        };

        let block_number = header.number;
        let block_hash = header.hash();

        println!("Block #{block_number} ({block_hash})");
    }

    println!("RPC client reconnected `{}` times", rpc.reconnect_count());

    Ok(())
}
