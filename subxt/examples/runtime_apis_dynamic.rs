use sp_keyring::AccountKeyring;
use subxt::dynamic::Value;
use subxt::{config::PolkadotConfig, OnlineClient};

#[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_small.scale")]
pub mod polkadot {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client to use:
    let api = OnlineClient::<PolkadotConfig>::new().await?;

    // Create a dynamically runtime API payload that calls the
    // `AccountNonceApi_account_nonce` function.
    let account = AccountKeyring::Alice.to_account_id();
    let runtime_api_call = subxt::dynamic::runtime_api_call(
        "AccountNonceApi",
        "account_nonce",
        vec![Value::from_bytes(account)],
    );

    // Submit the call to get back a result.
    let nonce = api
        .runtime_api()
        .at_latest()
        .await?
        .call(runtime_api_call)
        .await?;

    println!("Account nonce: {:#?}", nonce.to_value());
    Ok(())
}
