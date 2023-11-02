use futures::StreamExt;
use std::{iter, num::NonZeroU32};
use subxt::{client::LightClient, PolkadotConfig};

// Generate an interface that we can use from the node's metadata.
#[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_small.scale")]
pub mod polkadot {}

const POLKADOT_SPEC: &str = include_str!("../../artifacts/demo_chain_specs/polkadot.json");
const ASSET_HUB_SPEC: &str =
    include_str!("../../artifacts/demo_chain_specs/polkadot_asset_hub.json");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The smoldot logs are informative:
    tracing_subscriber::fmt::init();

    // Connecting to a parachain is a multi step process.

    // Step 1. Construct a new smoldot client.
    let mut client = subxt_lightclient::Client::new(subxt_lightclient::DefaultPlatform::new(
        "subxt-example-light-client".into(),
        "version-0".into(),
    ));

    // Step 2. Connect to the relay chain of the parachain. For this example, the Polkadot relay chain.
    let polkadot_connection = client
        .add_chain(subxt_lightclient::AddChainConfig {
            specification: POLKADOT_SPEC,
            json_rpc: subxt_lightclient::AddChainConfigJsonRpc::Enabled {
                max_pending_requests: NonZeroU32::new(128).unwrap(),
                max_subscriptions: 1024,
            },
            potential_relay_chains: iter::empty(),
            database_content: "",
            user_data: (),
        })
        .expect("Light client chain added with valid spec; qed");
    let polkadot_json_rpc_responses = polkadot_connection
        .json_rpc_responses
        .expect("Light client configured with json rpc enabled; qed");
    let polkadot_chain_id = polkadot_connection.chain_id;

    println!("Added Polkadot relay chain");

    // Step 3. Connect to the parachain. For this example, the Asset hub parachain.
    let assethub_connection = client
        .add_chain(subxt_lightclient::AddChainConfig {
            specification: ASSET_HUB_SPEC,
            json_rpc: subxt_lightclient::AddChainConfigJsonRpc::Enabled {
                max_pending_requests: NonZeroU32::new(128).unwrap(),
                max_subscriptions: 1024,
            },
            // The chain specification of the asset hub parachain mentions that the identifier
            // of its relay chain is `polkadot`.
            potential_relay_chains: [polkadot_chain_id].into_iter(),
            database_content: "",
            user_data: (),
        })
        .expect("Light client chain added with valid spec; qed");
    let parachain_json_rpc_responses = assethub_connection
        .json_rpc_responses
        .expect("Light client configured with json rpc enabled; qed");
    let parachain_chain_id = assethub_connection.chain_id;

    println!("Added AssetHub parachain");

    // Step 4. Turn the smoldot client into a subxt light client.
    let polkadot_api = LightClient::<PolkadotConfig>::builder()
        .build_from_raw(
            client,
            // The smoldot client is configured with two json rpc objects, one for the relay chain
            // and one for the parachain.
            [
                subxt_lightclient::AddedChain {
                    chain_id: polkadot_chain_id,
                    rpc_responses: polkadot_json_rpc_responses,
                },
                subxt_lightclient::AddedChain {
                    chain_id: parachain_chain_id,
                    rpc_responses: parachain_json_rpc_responses,
                },
            ]
            .into_iter(),
            // This client is responsible for talking with the relay chain.
            polkadot_chain_id,
        )
        .await?;

    // Step 5. Obtain a client to target the parachain.
    let parachain_api = polkadot_api
        .target_chain::<PolkadotConfig>(parachain_chain_id)
        .await?;

    // Step 6. Subscribe to the finalized blocks of the chains.
    let polkadot_sub = polkadot_api
        .blocks()
        .subscribe_finalized()
        .await?
        .map(|block| ("Polkadot", block));
    let parachain_sub = parachain_api
        .blocks()
        .subscribe_finalized()
        .await?
        .map(|block| ("AssetHub", block));
    let mut stream_combinator = futures::stream::select(polkadot_sub, parachain_sub);

    while let Some((chain, block)) = stream_combinator.next().await {
        let block = block?;

        println!("     Chain {:?} hash={:?}", chain, block.hash());
    }

    Ok(())
}
