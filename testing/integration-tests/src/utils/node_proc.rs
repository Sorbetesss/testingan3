// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use std::ffi::{OsStr, OsString};
use std::sync::Arc;
use substrate_runner::SubstrateNode;
use subxt::{
    backend::{legacy, rpc, unstable},
    Config, OnlineClient,
};

#[cfg(feature = "unstable-light-client")]
use subxt::client::{LightClient, LightClientBuilder};

/// Spawn a local substrate node for testing subxt.
pub struct TestNodeProcess<R: Config> {
    // Keep a handle to the node; once it's dropped the node is killed.
    proc: SubstrateNode,

    #[cfg(not(feature = "unstable-light-client"))]
    client: OnlineClient<R>,

    #[cfg(feature = "unstable-light-client")]
    client: LightClient<R>,
}

impl<R> TestNodeProcess<R>
where
    R: Config,
{
    /// Construct a builder for spawning a test node process.
    pub fn build<P>(paths: &[P]) -> TestNodeProcessBuilder
    where
        P: AsRef<OsStr> + Clone,
    {
        TestNodeProcessBuilder::new(paths)
    }

    /// Hand back an RPC client connected to the test node which exposes the legacy RPC methods.
    pub async fn legacy_rpc_methods(&self) -> legacy::LegacyRpcMethods<R> {
        let rpc_client = self.rpc_client().await;
        legacy::LegacyRpcMethods::new(rpc_client)
    }

    /// Hand back an RPC client connected to the test node which exposes the unstable RPC methods.
    pub async fn unstable_rpc_methods(&self) -> unstable::UnstableRpcMethods<R> {
        let rpc_client = self.rpc_client().await;
        unstable::UnstableRpcMethods::new(rpc_client)
    }

    async fn rpc_client(&self) -> rpc::RpcClient {
        let url = format!("ws://127.0.0.1:{}", self.proc.ws_port());
        rpc::RpcClient::from_url(url)
            .await
            .expect("Unable to connect RPC client to test node")
    }

    /// Returns the subxt client connected to the running node.
    #[cfg(not(feature = "unstable-light-client"))]
    pub fn client(&self) -> OnlineClient<R> {
        self.client.clone()
    }

    /// Returns the subxt client connected to the running node.
    #[cfg(feature = "unstable-light-client")]
    pub fn client(&self) -> LightClient<R> {
        self.client.clone()
    }
}

/// Construct a test node process.
pub struct TestNodeProcessBuilder {
    node_paths: Vec<OsString>,
    authority: Option<String>,
}

impl TestNodeProcessBuilder {
    pub fn new<P>(node_paths: &[P]) -> TestNodeProcessBuilder
    where
        P: AsRef<OsStr>,
    {
        // Check that paths are valid and build up vec.
        let mut paths = Vec::new();
        for path in node_paths {
            let path = path.as_ref();
            paths.push(path.to_os_string())
        }

        Self {
            node_paths: paths,
            authority: None,
        }
    }

    /// Set the authority dev account for a node in validator mode e.g. --alice.
    pub fn with_authority(&mut self, account: String) -> &mut Self {
        self.authority = Some(account);
        self
    }

    /// Spawn the substrate node at the given path, and wait for rpc to be initialized.
    pub async fn spawn<R>(self) -> Result<TestNodeProcess<R>, String>
    where
        R: Config,
    {
        let mut node_builder = SubstrateNode::builder();

        node_builder.binary_paths(&self.node_paths);

        if let Some(authority) = &self.authority {
            node_builder.arg(authority.to_lowercase());
        }

        // Spawn the node and retrieve a URL to it:
        let proc = node_builder.spawn().map_err(|e| e.to_string())?;
        let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

        #[cfg(feature = "unstable-light-client")]
        let client = build_light_client(&proc).await;

        #[cfg(feature = "unstable-backend-client")]
        let client = build_unstable_client(&proc).await;

        #[cfg(all(
            not(feature = "unstable-light-client"),
            not(feature = "unstable-backend-client")
        ))]
        let client = build_legacy_client(&proc).await;

        match client {
            Ok(client) => Ok(TestNodeProcess { proc, client }),
            Err(err) => Err(format!("Failed to connect to node rpc at {ws_url}: {err}")),
        }
    }
}

#[cfg(all(
    not(feature = "unstable-light-client"),
    not(feature = "unstable-backend-client")
))]
async fn build_legacy_client<T: Config>(proc: &SubstrateNode) -> Result<OnlineClient<T>, String> {
    let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

    let rpc_client = rpc::RpcClient::from_url(ws_url)
        .await
        .map_err(|e| format!("Cannot construct RPC client: {e}"))?;
    let backend = legacy::LegacyBackend::new(rpc_client);
    let client = OnlineClient::from_backend(Arc::new(backend))
        .await
        .map_err(|e| format!("Cannot construct OnlineClient from backend: {e}"))?;

    Ok(client)
}

#[cfg(feature = "unstable-backend-client")]
async fn build_unstable_client<T: Config>(proc: &SubstrateNode) -> Result<OnlineClient<T>, String> {
    let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

    let rpc_client = rpc::RpcClient::from_url(ws_url)
        .await
        .map_err(|e| format!("Cannot construct RPC client: {e}"))?;

    let (backend, mut driver) = unstable::UnstableBackend::builder().build(rpc_client);

    // The unstable backend needs driving:
    tokio::spawn(async move {
        use futures::StreamExt;
        while let Some(val) = driver.next().await {
            if let Err(e) = val {
                eprintln!("Error driving unstable backend: {e}");
                break;
            }
        }
    });

    let client = OnlineClient::from_backend(Arc::new(backend))
        .await
        .map_err(|e| format!("Cannot construct OnlineClient from backend: {e}"))?;

    Ok(client)
}

#[cfg(feature = "unstable-light-client")]
async fn build_light_client<T: Config>(proc: &SubstrateNode) -> Result<LightClient<T>, String> {
    // RPC endpoint.
    let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

    // Step 1. Wait for a few blocks to be produced using the subxt client.
    let client = OnlineClient::<T>::from_url(ws_url.clone())
        .await
        .map_err(|err| format!("Failed to connect to node rpc at {ws_url}: {err}"))?;

    super::wait_for_blocks(&client).await;

    // Step 2. Construct the light client.
    // P2p bootnode.
    let bootnode = format!(
        "/ip4/127.0.0.1/tcp/{}/p2p/{}",
        proc.p2p_port(),
        proc.p2p_address()
    );

    LightClientBuilder::new()
        .bootnodes([bootnode.as_str()])
        .build_from_url(ws_url.as_str())
        .await
        .map_err(|e| format!("Failed to construct light client {}", e.to_string()))
}
