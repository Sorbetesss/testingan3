#![allow(missing_docs)]
use subxt::{OnlineClient, PolkadotConfig};

#[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_full.scale")]
pub mod polkadot {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new API client, configured to talk to Polkadot nodes.
    let api = OnlineClient::<PolkadotConfig>::new().await?;

    // Build a storage query to iterate over account information.
    let storage_query = polkadot::storage().system().account_iter();

    // Get back an iterator of results (here, we are fetching 10 items at
    // a time from the node, but we always iterate over one at a time).
    let mut results = api.storage().at_latest().await?.iter(storage_query).await?;

    while let Some(Ok(kv)) = results.next().await {
        println!("Keys decoded: {:?}", kv.keys);
        println!("Key: 0x{}", hex::encode(&kv.key_bytes));
        println!("Value: {:?}", kv.value);
    }

    Ok(())
}
