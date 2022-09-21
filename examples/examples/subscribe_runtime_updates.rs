// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! To run this example, a local polkadot node should be running. Example verified against polkadot polkadot 0.9.25-5174e9ae75b.
//!
//! E.g.
//! ```bash
//! curl "https://github.com/paritytech/polkadot/releases/download/v0.9.25/polkadot" --output /usr/local/bin/polkadot --location
//! polkadot --dev --tmp
//! ```

use std::time::Duration;
use subxt::{client::UpgradeResult, OnlineClient, PolkadotConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create a client to use:
    let api = OnlineClient::<PolkadotConfig>::new().await?;

    // Start a new tokio task to perform the runtime updates while
    // utilizing the API for other use cases.
    let updater = api.subscribe_to_updates();
    tokio::spawn(async move {
        let mut update_stream = updater.runtime_updates().await.unwrap();

        while let Some(Ok(update)) = update_stream.next().await {
            let version = update.runtime_version.spec_version;

            match updater.apply_update(update).await {
                Ok(UpgradeResult::Success) => {
                    println!("Upgrade to version: {} successful", version)
                }
                Ok(reason) => {
                    println!("Upgrade to version: {} failed {:?}", version, reason)
                }
                Err(e) => {
                    println!(
                        "Upgrade failed {:?} (the websocket connection is probably gone)",
                        e
                    );
                    return;
                }
            };
        }
    });

    // If this client is kept in use a while, it'll update its metadata and such
    // as needed when the node it's pointed at updates.
    tokio::time::sleep(Duration::from_secs(10_000)).await;

    Ok(())
}
