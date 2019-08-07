// Copyright 2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-subxt.
//
// subxt is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// subxt is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with substrate-subxt.  If not, see <http://www.gnu.org/licenses/>.

use crate::{
    error::Error,
    metadata::Metadata,
    ExtrinsicSuccess,
};
use futures::{
    future::{
        self,
        Future,
        IntoFuture,
    },
    stream::Stream,
};
use jsonrpc_core_client::{
    RpcChannel,
    RpcError,
    TypedSubscriptionStream,
};
use log;
use num_traits::bounds::Bounded;
use parity_scale_codec::{
    Codec,
    Decode,
    Encode,
};

use runtime_metadata::RuntimeMetadataPrefixed;
use runtime_primitives::{
    generic::UncheckedExtrinsic,
    traits::{
        Hash as _,
        SignedExtension,
    },
};
use serde::{
    self,
    de::Error as DeError,
    Deserialize,
};
use srml_system::EventRecord;
use std::convert::TryInto;
use substrate_primitives::{
    blake2_256,
    storage::{
        StorageChangeSet,
        StorageKey,
    },
    twox_128,
    Pair,
};
use substrate_rpc::{
    author::AuthorClient,
    chain::{
        number::NumberOrHex,
        ChainClient,
    },
    state::StateClient,
};
use transaction_pool::txpool::watcher::Status;

/// Copy of runtime_primitives::OpaqueExtrinsic to allow a local Deserialize impl
#[derive(PartialEq, Eq, Clone, Default, Encode, Decode)]
pub struct OpaqueExtrinsic(pub Vec<u8>);

impl std::fmt::Debug for OpaqueExtrinsic {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            fmt,
            "{}",
            substrate_primitives::hexdisplay::HexDisplay::from(&self.0)
        )
    }
}

impl<'a> serde::Deserialize<'a> for OpaqueExtrinsic {
    fn deserialize<D>(de: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let r = substrate_primitives::bytes::deserialize(de)?;
        Decode::decode(&mut &r[..])
            .map_err(|e| DeError::custom(format!("Decode error: {}", e)))
    }
}

/// Copy of runtime_primitives::generic::Block with Deserialize implemented
#[derive(PartialEq, Eq, Clone, Encode, Decode, Debug, Deserialize)]
pub struct Block {
    // not included: pub header: Header,
    /// The accompanying extrinsics.
    pub extrinsics: Vec<OpaqueExtrinsic>,
}

/// Copy of runtime_primitives::generic::SignedBlock with Deserialize implemented
#[derive(PartialEq, Eq, Clone, Encode, Decode, Debug, Deserialize)]
pub struct SignedBlock {
    /// Full block.
    pub block: Block,
}

/// Client for substrate rpc interfaces
pub struct Rpc<T: srml_system::Trait, SE: SignedExtension> {
    _marker: std::marker::PhantomData<SE>,
    state: StateClient<T::Hash>,
    chain: ChainClient<T::BlockNumber, T::Hash, (), SignedBlock>,
    author: AuthorClient<T::Hash, T::Hash>,
}

/// Allows connecting to all inner interfaces on the same RpcChannel
impl<T: srml_system::Trait, SE: SignedExtension> From<RpcChannel> for Rpc<T, SE> {
    fn from(channel: RpcChannel) -> Self {
        Self {
            _marker: std::marker::PhantomData,
            state: channel.clone().into(),
            chain: channel.clone().into(),
            author: channel.into(),
        }
    }
}

impl<T: srml_system::Trait, SE: SignedExtension> Rpc<T, SE> {
    /// Fetch a storage key
    pub fn storage<V: Decode>(
        &self,
        key: StorageKey,
    ) -> impl Future<Item = Option<V>, Error = Error> {
        self.state
            .storage(key, None)
            .map(|data| {
                data.map(|d| Decode::decode(&mut &d.0[..]).expect("Valid storage key"))
            })
            .map_err(Into::into)
    }

    /// Fetch the genesis hash
    pub fn genesis_hash(&self) -> impl Future<Item = T::Hash, Error = Error> {
        let block_zero = T::BlockNumber::min_value();
        self.chain
            .block_hash(Some(NumberOrHex::Number(block_zero)))
            .map_err(Into::into)
            .and_then(|genesis_hash| {
                future::result(genesis_hash.ok_or("Genesis hash not found".into()))
            })
    }

    /// Fetch the metadata
    pub fn metadata(&self) -> impl Future<Item = Metadata, Error = Error> {
        self.state
            .metadata(None)
            .map(|bytes| Decode::decode(&mut &bytes[..]).unwrap())
            .map_err(Into::into)
            .and_then(|meta: RuntimeMetadataPrefixed| {
                future::result(meta.try_into().map_err(|err| format!("{:?}", err).into()))
            })
    }
}

impl<T: srml_system::Trait, SE: SignedExtension> Rpc<T, SE> {
    /// Subscribe to substrate System Events
    fn subscribe_events(
        &self,
    ) -> impl Future<Item = TypedSubscriptionStream<StorageChangeSet<T::Hash>>, Error = RpcError>
    {
        let events_key = b"System Events";
        let storage_key = twox_128(events_key);
        log::debug!("Events storage key {:?}", storage_key);

        self.state
            .subscribe_storage(Some(vec![StorageKey(storage_key.to_vec())]))
    }

    /// Submit an extrinsic, waiting for it to be finalized.
    /// If successful, returns the block hash.
    fn submit_and_watch<C, P>(
        self,
        extrinsic: UncheckedExtrinsic<T::AccountId, C, P::Signature, SE>,
    ) -> impl Future<Item = T::Hash, Error = Error>
    where
        C: Codec + Send,
        P: Pair,
        P::Public: Into<T::AccountId>,
        P::Signature: Codec,
    {
        self.author
            .watch_extrinsic(extrinsic.encode().into())
            .map_err(Into::into)
            .and_then(|stream| {
                stream
                    .filter_map(|status| {
                        match status {
                            Status::Future | Status::Ready | Status::Broadcast(_) => None, // ignore in progress extrinsic for now
                            Status::Finalized(block_hash) => Some(Ok(block_hash)),
                            Status::Usurped(_) => Some(Err("Extrinsic Usurped".into())),
                            Status::Dropped => Some(Err("Extrinsic Dropped".into())),
                            Status::Invalid => Some(Err("Extrinsic Invalid".into())),
                        }
                    })
                    .into_future()
                    .map_err(|(e, _)| e.into())
                    .and_then(|(result, _)| {
                        result
                            .ok_or(Error::from("Stream terminated"))
                            .and_then(|r| r)
                            .into_future()
                    })
            })
    }

    /// Create and submit an extrinsic and return corresponding Event if successful
    pub fn create_and_submit_extrinsic<C, P>(
        self,
        signer: P,
        call: C,
        extra: SE,
        account_nonce: T::Index,
        genesis_hash: T::Hash,
    ) -> impl Future<Item = ExtrinsicSuccess<T>, Error = Error>
    where
        C: Codec + Send,
        P: Pair,
        P::Public: Into<T::AccountId>,
        P::Signature: Codec,
    {
        let events = self.subscribe_events().map_err(Into::into);
        events.and_then(move |events| {
            let extrinsic = Self::create_and_sign_extrinsic(
                account_nonce,
                call,
                genesis_hash,
                &signer,
                extra,
            );
            let ext_hash = T::Hashing::hash_of(&extrinsic);

            log::info!("Submitting Extrinsic `{:?}`", ext_hash);

            let chain = self.chain.clone();
            self.submit_and_watch::<C, P>(extrinsic)
                .and_then(move |bh| {
                    log::info!("Fetching block {:?}", bh);
                    chain
                        .block(Some(bh))
                        .map(move |b| (bh, b))
                        .map_err(Into::into)
                })
                .and_then(|(h, b)| {
                    b.ok_or(format!("Failed to find block {:?}", h).into())
                        .map(|b| (h, b))
                        .into_future()
                })
                .and_then(move |(bh, sb)| {
                    log::info!(
                        "Found block {:?}, with {} extrinsics",
                        bh,
                        sb.block.extrinsics.len()
                    );
                    wait_for_block_events::<T>(ext_hash, &sb, bh, events)
                })
        })
    }

    /// Creates and signs an Extrinsic for the supplied `Call`
    fn create_and_sign_extrinsic<C, P>(
        index: T::Index,
        function: C,
        genesis_hash: T::Hash,
        signer: &P,
        extra: SE,
    ) -> UncheckedExtrinsic<T::AccountId, C, P::Signature, SE>
    where
        C: Encode + Send,
        P: Pair,
        P::Public: Into<T::AccountId>,
        P::Signature: Codec,
    {
        log::info!(
            "Creating Extrinsic with genesis hash {:?} and account nonce {:?}",
            genesis_hash,
            index
        );

        let raw_payload = (function, extra.clone(), genesis_hash);
        let signature = raw_payload.using_encoded(|payload| {
            if payload.len() > 256 {
                signer.sign(&blake2_256(payload)[..])
            } else {
                signer.sign(payload)
            }
        });

        UncheckedExtrinsic::new_signed(
            raw_payload.0,
            signer.public().into(),
            signature.into(),
            extra,
        )
    }
}

/// Waits for events for the block triggered by the extrinsic
fn wait_for_block_events<T: srml_system::Trait>(
    ext_hash: T::Hash,
    signed_block: &SignedBlock,
    block_hash: T::Hash,
    events_stream: TypedSubscriptionStream<StorageChangeSet<T::Hash>>,
) -> impl Future<Item = ExtrinsicSuccess<T>, Error = Error> {
    let ext_index = signed_block
        .block
        .extrinsics
        .iter()
        .position(|ext| {
            let hash = T::Hashing::hash_of(ext);
            hash == ext_hash
        })
        .ok_or(format!("Failed to find Extrinsic with hash {:?}", ext_hash).into())
        .into_future();

    let block_hash = block_hash.clone();
    let block_events = events_stream
        .filter(move |event| event.block == block_hash)
        .into_future()
        .map_err(|(e, _)| e.into())
        .and_then(|(change_set, _)| {
            match change_set {
                None => future::ok(Vec::new()),
                Some(change_set) => {
                    let events = change_set
                        .changes
                        .iter()
                        .filter_map(|(_key, data)| {
                            data.as_ref().map(|data| Decode::decode(&mut &data.0[..]))
                        })
                        .collect::<Result<Vec<Vec<EventRecord<T::Event, T::Hash>>>, _>>()
                        .map(|events| events.into_iter().flat_map(|es| es).collect())
                        .map_err(Into::into);
                    future::result(events)
                }
            }
        });

    block_events
        .join(ext_index)
        .map(move |(events, ext_index)| {
            let events: Vec<T::Event> = events
                .iter()
                .filter_map(|e| {
                    if let srml_system::Phase::ApplyExtrinsic(i) = e.phase {
                        if i as usize == ext_index {
                            Some(e.event.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect::<Vec<T::Event>>();
            ExtrinsicSuccess {
                block: block_hash,
                extrinsic: ext_hash,
                events,
            }
        })
}
