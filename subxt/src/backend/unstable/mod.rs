// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! This module will expose a backend implementation based on the new APIs
//! described at <https://github.com/paritytech/json-rpc-interface-spec/>. See
//! [`rpc_methods`] for the raw API calls.
//!
//! # Warning
//!
//! Everything in this module is **unstable**, meaning that it could change without
//! warning at any time.

mod follow_stream;
mod follow_stream_driver;
mod follow_stream_unpin;
mod storage_items;

pub mod rpc_methods;

use self::rpc_methods::{
    FollowEvent, MethodResponse, RuntimeEvent, StorageQuery, StorageQueryType, StorageResultType,
};
use crate::backend::{
    rpc::RpcClient, Backend, BlockRef, BlockRefT, RuntimeVersion, StorageResponse, StreamOf,
    StreamOfResults, TransactionStatus,
};
use crate::config::BlockHash;
use crate::error::{Error, RpcError};
use crate::Config;
use async_trait::async_trait;
use follow_stream_driver::{FollowStreamDriver, FollowStreamDriverHandle};
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::task::Poll;
use storage_items::StorageItems;

// Expose the RPC methods.
pub use rpc_methods::UnstableRpcMethods;

/// Configure and build an [`UnstableBackend`].
pub struct UnstableBackendBuilder<T> {
    max_block_life: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<T: Config> Default for UnstableBackendBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Config> UnstableBackendBuilder<T> {
    /// Create a new [`UnstableBackendBuilder`].
    pub fn new() -> Self {
        Self {
            max_block_life: usize::MAX,
            _marker: std::marker::PhantomData,
        }
    }

    /// The age of a block is defined here as the difference between the current finalized block number
    /// and the block number of a given block. Once the difference equals or exceeds the number given
    /// here, the block is unpinned.
    ///
    /// By default, we will never automatically unpin blocks, but if the number of pinned blocks that we
    /// keep hold of exceeds the number that the server can tolerate, then a `stop` event is generated and
    /// we are forced to resubscribe, losing any pinned blocks.
    pub fn max_block_life(mut self, max_block_life: usize) -> Self {
        self.max_block_life = max_block_life;
        self
    }

    /// Given an [`RpcClient`] to use to make requests, this returns a tuple of an [`UnstableBackend`],
    /// which implements the [`Backend`] trait, and an [`UnstableBackendDriver`] which must be polled in
    /// order for the backend to make progress.
    pub fn build(self, client: RpcClient) -> (UnstableBackend<T>, UnstableBackendDriver<T>) {
        // Construct the underlying follow_stream layers:
        let rpc_methods = UnstableRpcMethods::new(client);
        let follow_stream =
            follow_stream::FollowStream::<T::Hash>::from_methods(rpc_methods.clone());
        let follow_stream_unpin = follow_stream_unpin::FollowStreamUnpin::<T::Hash>::from_methods(
            follow_stream,
            rpc_methods.clone(),
            self.max_block_life,
        );
        let follow_stream_driver = FollowStreamDriver::new(follow_stream_unpin);

        // Wrap these into the backend and driver that we'll expose.
        let backend = UnstableBackend {
            methods: rpc_methods,
            follow_handle: follow_stream_driver.handle(),
        };
        let driver = UnstableBackendDriver {
            driver: follow_stream_driver,
        };

        (backend, driver)
    }
}

/// Driver for the [`UnstableBackend`]. This must be polled in order for the
/// backend to make progress.
#[derive(Debug)]
pub struct UnstableBackendDriver<T: Config> {
    driver: FollowStreamDriver<T::Hash>,
}

impl<T: Config> Stream for UnstableBackendDriver<T> {
    type Item = <FollowStreamDriver<T::Hash> as Stream>::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.driver.poll_next_unpin(cx)
    }
}

/// The unstable backend.
#[derive(Debug, Clone)]
pub struct UnstableBackend<T: Config> {
    // RPC methods we'll want to call:
    methods: UnstableRpcMethods<T>,
    // A handle to the chainHead_follow subscription:
    follow_handle: FollowStreamDriverHandle<T::Hash>,
}

impl<T: Config> UnstableBackend<T> {
    /// Configure and construct an [`UnstableBackend`] and the associated [`UnstableBackendDriver`].
    pub fn builder() -> UnstableBackendBuilder<T> {
        UnstableBackendBuilder::new()
    }

    /// Stream block headers based on the provided filter fn
    async fn stream_headers<F, I>(
        &self,
        f: F,
    ) -> Result<StreamOfResults<(T::Header, BlockRef<T::Hash>)>, Error>
    where
        F: Fn(FollowEvent<follow_stream_unpin::BlockRef<T::Hash>>) -> I + Copy + Send + 'static,
        I: IntoIterator<Item = follow_stream_unpin::BlockRef<T::Hash>> + Send + 'static,
        <I as IntoIterator>::IntoIter: Send,
    {
        let sub_id = get_subscription_id(&self.follow_handle).await?;
        let sub_id = Arc::new(sub_id);
        let methods = self.methods.clone();
        let headers = self.follow_handle.subscribe().events().flat_map(move |ev| {
            let sub_id = sub_id.clone();
            let methods = methods.clone();

            let block_refs = f(ev).into_iter();

            futures::stream::iter(block_refs).filter_map(move |block_ref| {
                let sub_id = sub_id.clone();
                let methods = methods.clone();

                async move {
                    let res = methods
                        .chainhead_unstable_header(&sub_id, block_ref.hash())
                        .await
                        .transpose()?;

                    let header = match res {
                        Ok(header) => header,
                        Err(e) => return Some(Err(e)),
                    };

                    Some(Ok((header, block_ref.into())))
                }
            })
        });

        Ok(StreamOf(Box::pin(headers)))
    }
}

impl<Hash: BlockHash + 'static> BlockRefT for follow_stream_unpin::BlockRef<Hash> {}
impl<Hash: BlockHash + 'static> From<follow_stream_unpin::BlockRef<Hash>> for BlockRef<Hash> {
    fn from(b: follow_stream_unpin::BlockRef<Hash>) -> Self {
        BlockRef::new(b.hash(), b)
    }
}

impl<T: Config> super::sealed::Sealed for UnstableBackend<T> {}

#[async_trait]
impl<T: Config + Send + Sync + 'static> Backend<T> for UnstableBackend<T> {
    async fn storage_fetch_values(
        &self,
        keys: Vec<Vec<u8>>,
        at: T::Hash,
    ) -> Result<StreamOfResults<StorageResponse>, Error> {
        let queries = keys.iter().map(|key| StorageQuery {
            key: &**key,
            query_type: StorageQueryType::Value,
        });

        let storage_items =
            StorageItems::from_methods(queries, at, &self.follow_handle, self.methods.clone())
                .await?;

        let storage_result_stream = storage_items.filter_map(|val| async move {
            let val = match val {
                Ok(val) => val,
                Err(e) => return Some(Err(e)),
            };

            let StorageResultType::Value(result) = val.result else {
                return None;
            };
            Some(Ok(StorageResponse {
                key: val.key.0,
                value: result.0,
            }))
        });

        Ok(StreamOf(Box::pin(storage_result_stream)))
    }

    async fn storage_fetch_descendant_keys(
        &self,
        key: Vec<u8>,
        at: T::Hash,
    ) -> Result<StreamOfResults<Vec<u8>>, Error> {
        // Ask for hashes, and then just ignore them and return the keys that come back.
        let query = StorageQuery {
            key: &*key,
            query_type: StorageQueryType::DescendantsHashes,
        };

        let storage_items = StorageItems::from_methods(
            std::iter::once(query),
            at,
            &self.follow_handle,
            self.methods.clone(),
        )
        .await?;

        let storage_result_stream = storage_items.map(|val| val.map(|v| v.key.0));
        Ok(StreamOf(Box::pin(storage_result_stream)))
    }

    async fn storage_fetch_descendant_values(
        &self,
        key: Vec<u8>,
        at: T::Hash,
    ) -> Result<StreamOfResults<StorageResponse>, Error> {
        let query = StorageQuery {
            key: &*key,
            query_type: StorageQueryType::DescendantsValues,
        };

        let storage_items = StorageItems::from_methods(
            std::iter::once(query),
            at,
            &self.follow_handle,
            self.methods.clone(),
        )
        .await?;

        let storage_result_stream = storage_items.filter_map(|val| async move {
            let val = match val {
                Ok(val) => val,
                Err(e) => return Some(Err(e)),
            };

            let StorageResultType::Value(result) = val.result else {
                return None;
            };
            Some(Ok(StorageResponse {
                key: val.key.0,
                value: result.0,
            }))
        });

        Ok(StreamOf(Box::pin(storage_result_stream)))
    }

    async fn genesis_hash(&self) -> Result<T::Hash, Error> {
        self.methods.chainspec_v1_genesis_hash().await
    }

    async fn block_header(&self, at: T::Hash) -> Result<Option<T::Header>, Error> {
        let sub_id = get_subscription_id(&self.follow_handle).await?;
        self.methods.chainhead_unstable_header(&sub_id, at).await
    }

    async fn block_body(&self, at: T::Hash) -> Result<Option<Vec<Vec<u8>>>, Error> {
        let sub_id = get_subscription_id(&self.follow_handle).await?;

        // Subscribe to the body response and get our operationId back.
        let follow_events = self.follow_handle.subscribe().events();
        let status = self.methods.chainhead_unstable_body(&sub_id, at).await?;
        let operation_id = match status {
            MethodResponse::LimitReached => {
                return Err(RpcError::request_rejected("limit reached").into())
            }
            MethodResponse::Started(s) => s.operation_id,
        };

        // Wait for the response to come back with the correct operationId.
        let mut exts_stream = follow_events.filter_map(|ev| {
            let FollowEvent::OperationBodyDone(body) = ev else {
                return std::future::ready(None);
            };
            if body.operation_id != operation_id {
                return std::future::ready(None);
            }
            let exts: Vec<_> = body.value.into_iter().map(|ext| ext.0).collect();
            std::future::ready(Some(exts))
        });

        Ok(exts_stream.next().await)
    }

    async fn latest_finalized_block_ref(&self) -> Result<BlockRef<T::Hash>, Error> {
        let next_ref: Option<BlockRef<T::Hash>> = self
            .follow_handle
            .subscribe()
            .events()
            .filter_map(|ev| {
                let out = match ev {
                    FollowEvent::Initialized(init) => Some(init.finalized_block_hash.into()),
                    _ => None,
                };
                std::future::ready(out)
            })
            .next()
            .await;

        next_ref.ok_or_else(|| RpcError::SubscriptionDropped.into())
    }

    async fn current_runtime_version(&self) -> Result<RuntimeVersion, Error> {
        // Just start a stream of version infos, and return the first value we get from it.
        let runtime_version = self.stream_runtime_version().await?.next().await;
        match runtime_version {
            None => Err(Error::Rpc(RpcError::SubscriptionDropped)),
            Some(Err(e)) => Err(e),
            Some(Ok(version)) => Ok(version),
        }
    }

    async fn stream_runtime_version(&self) -> Result<StreamOfResults<RuntimeVersion>, Error> {
        // Keep track of runtime details announced in new blocks, and then when blocks
        // are finalized, find the latest of these that has runtime details, and clear the rest.
        let mut runtimes = HashMap::new();
        let runtime_stream = self
            .follow_handle
            .subscribe()
            .events()
            .filter_map(move |ev| {
                let output = match ev {
                    FollowEvent::Initialized(ev) => {
                        runtimes.clear();
                        ev.finalized_block_runtime
                    }
                    FollowEvent::NewBlock(ev) => {
                        if let Some(runtime) = ev.new_runtime {
                            runtimes.insert(ev.block_hash.hash(), runtime);
                        }
                        None
                    }
                    FollowEvent::Finalized(ev) => {
                        let next_runtime = ev
                            .finalized_block_hashes
                            .iter()
                            .rev()
                            .filter_map(|h| runtimes.get(&h.hash()).cloned())
                            .next();

                        runtimes.clear();
                        next_runtime
                    }
                    _ => None,
                };

                let runtime_event = match output {
                    None => return std::future::ready(None),
                    Some(ev) => ev,
                };

                let runtime_details = match runtime_event {
                    RuntimeEvent::Invalid(err) => {
                        return std::future::ready(Some(Err(Error::Other(err.error))))
                    }
                    RuntimeEvent::Valid(ev) => ev,
                };

                std::future::ready(Some(Ok(RuntimeVersion {
                    spec_version: runtime_details.spec.spec_version,
                    transaction_version: runtime_details.spec.transaction_version,
                })))
            });

        Ok(StreamOf(Box::pin(runtime_stream)))
    }

    async fn stream_all_block_headers(
        &self,
    ) -> Result<StreamOfResults<(T::Header, BlockRef<T::Hash>)>, Error> {
        self.stream_headers(|ev| match ev {
            FollowEvent::Initialized(ev) => Some(ev.finalized_block_hash),
            FollowEvent::NewBlock(ev) => Some(ev.block_hash),
            _ => None,
        })
        .await
    }

    async fn stream_best_block_headers(
        &self,
    ) -> Result<StreamOfResults<(T::Header, BlockRef<T::Hash>)>, Error> {
        self.stream_headers(|ev| match ev {
            FollowEvent::Initialized(ev) => Some(ev.finalized_block_hash),
            FollowEvent::BestBlockChanged(ev) => Some(ev.best_block_hash),
            _ => None,
        })
        .await
    }

    async fn stream_finalized_block_headers(
        &self,
    ) -> Result<StreamOfResults<(T::Header, BlockRef<T::Hash>)>, Error> {
        self.stream_headers(|ev| match ev {
            FollowEvent::Initialized(ev) => {
                vec![ev.finalized_block_hash]
            }
            FollowEvent::Finalized(ev) => ev.finalized_block_hashes,
            _ => vec![],
        })
        .await
    }

    async fn submit_transaction(
        &self,
        extrinsic: &[u8],
    ) -> Result<StreamOfResults<TransactionStatus<T::Hash>>, Error> {
        // We care about new and finalized block hashes.
        enum SeenBlock<Ref> {
            New(Ref),
            Finalized(Vec<Ref>),
        }
        enum SeenBlockMarker {
            New,
            Finalized,
        }

        // First, subscribe to all new and finalized block refs.
        // - we subscribe to new refs so that when we see `BestChainBlockIncluded`, we
        //   can try to return a block ref for the best block.
        // - we subscribe to finalized refs so that when we see `Finalized`, we can
        //   guarantee that when we return here, the finalized block we report has been
        //   reported from chainHead_follow already.
        let mut seen_blocks_sub = self.follow_handle.subscribe().events().filter_map(|ev| {
            std::future::ready(match ev {
                FollowEvent::NewBlock(ev) => Some(SeenBlock::New(ev.block_hash)),
                FollowEvent::Finalized(ev) => Some(SeenBlock::Finalized(ev.finalized_block_hashes)),
                _ => None,
            })
        });

        // Then, submit the transaction.
        let mut tx_progress = self
            .methods
            .transaction_unstable_submit_and_watch(extrinsic)
            .await?;

        let mut seen_blocks = HashMap::new();
        let mut done = false;

        // If we see the finalized event, we start waiting until we find a finalized block that
        // matches, so we can guarantee to return a pinned block hash and be properly in sync
        // with chainHead_follow.
        let mut finalized_hash: Option<T::Hash> = None;

        // Now we can attempt to associate tx events with pinned blocks.
        let tx_stream = futures::stream::poll_fn(move |cx| {
            loop {
                // Bail early if no more tx events; we don't want to keep polling for pinned blocks.
                if done {
                    return Poll::Ready(None);
                }

                // Make a note of new or finalized blocks that have come in since we started the TX.
                if let Poll::Ready(Some(seen_block)) = seen_blocks_sub.poll_next_unpin(cx) {
                    match seen_block {
                        SeenBlock::New(block_ref) => {
                            if finalized_hash.is_none() {
                                seen_blocks
                                    .insert(block_ref.hash(), (SeenBlockMarker::New, block_ref));
                            }
                        }
                        SeenBlock::Finalized(block_refs) => {
                            for block_ref in block_refs {
                                seen_blocks.insert(
                                    block_ref.hash(),
                                    (SeenBlockMarker::Finalized, block_ref),
                                );
                            }
                        }
                    }
                    continue;
                }

                // If we have a finalized hash, we are done looking for tx events and we are just waiting
                // for a pinned block with a matching hash (which must appear eventually given it's finalized).
                if let Some(hash) = &finalized_hash {
                    if let Some((SeenBlockMarker::Finalized, block_ref)) = seen_blocks.remove(hash)
                    {
                        // Found it! Hand back the event with a pinned block. We're done.
                        done = true;
                        let ev = TransactionStatus::InFinalizedBlock {
                            hash: block_ref.into(),
                        };
                        return Poll::Ready(Some(Ok(ev)));
                    } else {
                        // Keep waiting for more finalized blocks until we find it (get rid of any other block refs
                        // now, since none of them were what we were looking for anyway).
                        seen_blocks.clear();
                        continue;
                    }
                }

                // Otherwise, we are still watching for tx events:
                let ev = match tx_progress.poll_next_unpin(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(None) => {
                        done = true;
                        return Poll::Ready(None);
                    }
                    Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                    Poll::Ready(Some(Ok(ev))) => ev,
                };

                // When we get one, map it to the correct format (or for finalized ev, wait for the pinned block):
                let ev = match ev {
                    rpc_methods::TransactionStatus::Finalized { block } => {
                        // We'll wait until we have seen this hash, to try to guarantee
                        // that when we return this event, the corresponding block is
                        // pinned and accessible.
                        finalized_hash = Some(block.hash);
                        continue;
                    }
                    rpc_methods::TransactionStatus::BestChainBlockIncluded {
                        block: Some(block),
                    } => {
                        // Look up a pinned block ref if we can, else return a non-pinned
                        // block that likely isn't accessible. We have no guarantee that a best
                        // block on the node a tx was sent to will ever be known about on the
                        // chainHead_follow subscription.
                        let block_ref = match seen_blocks.get(&block.hash) {
                            Some((_, block_ref)) => block_ref.clone().into(),
                            None => BlockRef::from_hash(block.hash),
                        };
                        TransactionStatus::InBestBlock { hash: block_ref }
                    }
                    rpc_methods::TransactionStatus::BestChainBlockIncluded { block: None } => {
                        TransactionStatus::NoLongerInBestBlock
                    }
                    rpc_methods::TransactionStatus::Broadcasted { num_peers } => {
                        TransactionStatus::Broadcasted { num_peers }
                    }
                    rpc_methods::TransactionStatus::Dropped { error, .. } => {
                        TransactionStatus::Dropped { message: error }
                    }
                    rpc_methods::TransactionStatus::Error { error } => {
                        TransactionStatus::Dropped { message: error }
                    }
                    rpc_methods::TransactionStatus::Invalid { error } => {
                        TransactionStatus::Invalid { message: error }
                    }
                    rpc_methods::TransactionStatus::Validated => TransactionStatus::Validated,
                };
                return Poll::Ready(Some(Ok(ev)));
            }
        });

        Ok(StreamOf(Box::pin(tx_stream)))
    }

    async fn call(
        &self,
        method: &str,
        call_parameters: Option<&[u8]>,
        at: T::Hash,
    ) -> Result<Vec<u8>, Error> {
        let sub_id = get_subscription_id(&self.follow_handle).await?;

        // Subscribe to the body response and get our operationId back.
        let follow_events = self.follow_handle.subscribe().events();
        let call_parameters = call_parameters.unwrap_or(&[]);
        let status = self
            .methods
            .chainhead_unstable_call(&sub_id, at, method, call_parameters)
            .await?;
        let operation_id = match status {
            MethodResponse::LimitReached => {
                return Err(RpcError::request_rejected("limit reached").into())
            }
            MethodResponse::Started(s) => s.operation_id,
        };

        // Wait for the response to come back with the correct operationId.
        let mut call_data_stream = follow_events.filter_map(|ev| {
            let FollowEvent::OperationCallDone(body) = ev else {
                return std::future::ready(None);
            };
            if body.operation_id != operation_id {
                return std::future::ready(None);
            }
            std::future::ready(Some(body.output.0))
        });

        call_data_stream
            .next()
            .await
            .ok_or_else(|| RpcError::SubscriptionDropped.into())
    }
}

/// A helper to obtain a subscription ID.
async fn get_subscription_id<Hash: BlockHash>(
    follow_handle: &FollowStreamDriverHandle<Hash>,
) -> Result<String, Error> {
    let Some(sub_id) = follow_handle.subscribe().subscription_id().await else {
        return Err(RpcError::SubscriptionDropped.into());
    };

    Ok(sub_id)
}
