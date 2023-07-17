#![cfg(target_arch = "wasm32")]

use subxt::{config::PolkadotConfig,
    client::{LightClient, OfflineClientT, LightClientBuilder},
};
use futures_util::StreamExt;
use wasm_bindgen_test::*;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// Run the tests by calling:
//
// ```text
// wasm-pack test --firefox --headless`
// ```
//
// You'll need to have a substrate/polkadot node running:
//
// ```bash
// # Polkadot does not accept by default WebSocket connections to the P2P network.
// # Ensure `--listen-addr` is provided with valid ws adddress endpoint.
// # The `--node-key` provides a deterministic p2p address for the node.
// ./polkadot --dev --node-key 0000000000000000000000000000000000000000000000000000000000000001 --listen-addr /ip4/0.0.0.0/tcp/30333/ws
// ```
//
// Use the following to enable logs:
// ```
//  console_error_panic_hook::set_once();
//  tracing_wasm::set_as_global_default();
// ```

#[wasm_bindgen_test]
async fn wasm_ws_transport_works() {
    let client = subxt::client::OnlineClient::<PolkadotConfig>::from_url("ws://127.0.0.1:9944")
        .await
        .unwrap();

    let chain = client.rpc().system_chain().await.unwrap();
    assert_eq!(&chain, "Development");
}
