// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use std::ffi::{OsStr, OsString};
use substrate_runner::SubstrateNode;
use subxt::{Config, OnlineClient};

#[cfg(feature = "unstable-light-client")]
use std::sync::Arc;
#[cfg(feature = "unstable-light-client")]
use subxt::client::LightClientBuilder;

/// Spawn a local substrate node for testing subxt.
pub struct TestNodeProcess<R: Config> {
    // Keep a handle to the node; once it's dropped the node is killed.
    _proc: SubstrateNode,
    client: OnlineClient<R>,
}

impl<R> TestNodeProcess<R>
where
    R: Config,
{
    /// Construct a builder for spawning a test node process.
    pub fn build<S>(program: S) -> TestNodeProcessBuilder
    where
        S: AsRef<OsStr> + Clone,
    {
        TestNodeProcessBuilder::new(program)
    }

    /// Returns the subxt client connected to the running node.
    pub fn client(&self) -> OnlineClient<R> {
        self.client.clone()
    }
}

/// Construct a test node process.
pub struct TestNodeProcessBuilder {
    node_path: OsString,
    authority: Option<String>,
}

impl TestNodeProcessBuilder {
    pub fn new<P>(node_path: P) -> TestNodeProcessBuilder
    where
        P: AsRef<OsStr>,
    {
        Self {
            node_path: node_path.as_ref().into(),
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

        node_builder.binary_path(self.node_path);

        if let Some(authority) = &self.authority {
            node_builder.arg(authority.to_lowercase());
        }

        // Spawn the node and retrieve a URL to it:
        let proc = node_builder.spawn().map_err(|e| e.to_string())?;
        let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

        #[cfg(feature = "unstable-light-client")]
        let client = build_light_client(proc).await;

        // Connect to the node with a subxt client:
        #[cfg(not(feature = "unstable-light-client"))]
        let client = OnlineClient::from_url(ws_url.clone()).await;

        match client {
            Ok(client) => Ok(TestNodeProcess {
                _proc: proc,
                client,
            }),
            Err(err) => Err(format!("Failed to connect to node rpc at {ws_url}: {err}")),
        }
    }
}

#[cfg(feature = "unstable-light-client")]
async fn build_light_client<R: Config>(proc: SubstrateNode) -> Result<OnlineClient<R>, Error> {
    // RPC endpoint.
    let ws_url = format!("ws://127.0.0.1:{}", proc.ws_port());

    // P2p bootnode.
    let bootnode = format!(
        "/ip4/127.0.0.1/tcp/{}/p2p/{}",
        proc.p2p_port(),
        proc.p2p_address()
    );

    let light_client = LightClientBuilder::new()
        .bootnodes([bootnode.as_str()].into_iter())
        .build_from_url(ws_url.as_str())
        .await
        .map_err(|e| e.to_string())?;

    OnlineClient::from_rpc_client(Arc::new(light_client)).await
}
