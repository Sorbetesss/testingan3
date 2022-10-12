// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use crate::{
    client::OnlineClientT,
    error::Error,
    utils::PhantomDataSendSync,
    Config,
};
use derivative::Derivative;
use futures::{
    future::Either,
    stream,
    Stream,
    StreamExt,
};
use sp_runtime::traits::Header;
use std::future::Future;

/// A client for working with blocks.
#[derive(Derivative)]
#[derivative(Clone(bound = "Client: Clone"))]
pub struct BlocksClient<T, Client> {
    client: Client,
    _marker: PhantomDataSendSync<T>,
}

impl<T, Client> BlocksClient<T, Client> {
    /// Create a new [`BlocksClient`].
    pub fn new(client: Client) -> Self {
        Self {
            client,
            _marker: PhantomDataSendSync::new(),
        }
    }
}

impl<T, Client> BlocksClient<T, Client>
where
    T: Config,
    Client: OnlineClientT<T>,
{
    /// Subscribe to new best block headers.
    ///
    /// # Note
    ///
    /// This does not produce all the blocks from the chain, just the best blocks.
    /// The best block is selected by the consensus algorithm.
    /// This calls under the hood the `chain_subscribeNewHeads` RPC method, if you need
    /// a subscription of all the blocks please use the `chain_subscribeAllHeads` method.
    ///
    /// These blocks haven't necessarily been finalised yet. Prefer
    /// [`BlocksClient::subscribe_finalized_headers()`] if that is important.
    pub fn subscribe_headers(
        &self,
    ) -> impl Future<Output = Result<impl Stream<Item = Result<T::Header, Error>>, Error>>
           + Send
           + 'static {
        let client = self.client.clone();
        async move { client.rpc().subscribe_blocks().await }
    }

    /// Subscribe to finalized block headers.
    ///
    /// While the Substrate RPC method does not guarantee that all finalized block headers are
    /// provided, this function does.
    /// ```
    pub fn subscribe_finalized_headers(
        &self,
    ) -> impl Future<Output = Result<impl Stream<Item = Result<T::Header, Error>>, Error>>
           + Send
           + 'static {
        let client = self.client.clone();
        async move { subscribe_finalized_headers(client).await }
    }
}

async fn subscribe_finalized_headers<T, Client>(
    client: Client,
) -> Result<impl Stream<Item = Result<T::Header, Error>>, Error>
where
    T: Config,
    Client: OnlineClientT<T>,
{
    // Fetch the last finalised block details immediately, so that we'll get
    // all blocks after this one.
    let last_finalized_block_hash = client.rpc().finalized_head().await?;
    let last_finalized_block_num = client
        .rpc()
        .header(Some(last_finalized_block_hash))
        .await?
        .map(|h| (*h.number()).into());

    let sub = client.rpc().subscribe_finalized_blocks().await?;

    // Adjust the subscription stream to fill in any missing blocks.
    Ok(
        subscribe_to_block_headers_filling_in_gaps(client, last_finalized_block_num, sub)
            .boxed(),
    )
}

/// Note: This is exposed for testing but is not considered stable and may change
/// without notice in a patch release.
#[doc(hidden)]
pub fn subscribe_to_block_headers_filling_in_gaps<T, Client, S, E>(
    client: Client,
    mut last_block_num: Option<u64>,
    sub: S,
) -> impl Stream<Item = Result<T::Header, Error>> + Send
where
    T: Config,
    Client: OnlineClientT<T>,
    S: Stream<Item = Result<T::Header, E>> + Send,
    E: Into<Error> + Send + 'static,
{
    sub.flat_map(move |s| {
        let client = client.clone();

        // Get the header, or return a stream containing just the error.
        let header = match s {
            Ok(header) => header,
            Err(e) => return Either::Left(stream::once(async { Err(e.into()) })),
        };

        // We want all previous details up to, but not including this current block num.
        let end_block_num = (*header.number()).into();

        // This is one after the last block we returned details for last time.
        let start_block_num = last_block_num.map(|n| n + 1).unwrap_or(end_block_num);

        // Iterate over all of the previous blocks we need headers for, ignoring the current block
        // (which we already have the header info for):
        let previous_headers = stream::iter(start_block_num..end_block_num)
            .then(move |n| {
                let rpc = client.rpc().clone();
                async move {
                    let hash = rpc.block_hash(Some(n.into())).await?;
                    let header = rpc.header(hash).await?;
                    Ok::<_, Error>(header)
                }
            })
            .filter_map(|h| async { h.transpose() });

        // On the next iteration, we'll get details starting just after this end block.
        last_block_num = Some(end_block_num);

        // Return a combination of any previous headers plus the new header.
        Either::Right(previous_headers.chain(stream::once(async { Ok(header) })))
    })
}
