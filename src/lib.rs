// Copyright 2018-2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-subxt.
//
// ink! is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// ink! is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with substrate-subxt.  If not, see <http://www.gnu.org/licenses/>.

use crate::error::{Error, Result};
use futures::{
    future::{
        self,
        Future,
        IntoFuture,
    },
    stream::Stream,
};
use jsonrpc_core_client::{
    transports::ws,
    RpcChannel,
    RpcError,
    TypedSubscriptionStream,
};
use log;
use node_runtime::{
    Call,
    Runtime,
    UncheckedExtrinsic,
};
use parity_codec::{
    Decode,
    Encode,
};

use runtime_primitives::generic::Era;
use runtime_support::StorageMap;
use serde::{
    self,
    de::Error as DeError,
    Deserialize,
};
use substrate_primitives::{
    blake2_256,
    sr25519::Pair,
    storage::{
        StorageChangeSet,
        StorageKey,
    },
    twox_128,
    Pair as _,
    H256,
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
use url::Url;

mod error;
mod rpc;

type AccountId = <Runtime as srml_system::Trait>::AccountId;
type BlockNumber = <Runtime as srml_system::Trait>::BlockNumber;
type Index = <Runtime as srml_system::Trait>::Index;
type Hash = <Runtime as srml_system::Trait>::Hash;
type Event = <Runtime as srml_system::Trait>::Event;
type EventRecord = srml_system::EventRecord<Event, Hash>;

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
            .ok_or(DeError::custom("Invalid value passed into decode"))
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

/// Captures data for when an extrinsic is successfully included in a block
#[derive(Debug)]
pub struct ExtrinsicSuccess {
    pub block: Hash,
    pub extrinsic: Hash,
    pub events: Vec<Event>,
}

/// Client for substrate rpc interfaces
struct Rpc {
    state: StateClient<Hash>,
    chain: ChainClient<BlockNumber, Hash, (), SignedBlock>,
    author: AuthorClient<Hash, Hash>,
}

/// Allows connecting to all inner interfaces on the same RpcChannel
impl From<RpcChannel> for Rpc {
    fn from(channel: RpcChannel) -> Rpc {
        Rpc {
            state: channel.clone().into(),
            chain: channel.clone().into(),
            author: channel.into(),
        }
    }
}

impl Rpc {
    /// Fetch the latest nonce for the given `AccountId`
    fn fetch_nonce(
        &self,
        account: &AccountId,
    ) -> impl Future<Item = u64, Error = RpcError> {
        let account_nonce_key = <srml_system::AccountNonce<Runtime>>::key_for(account);
        let storage_key = blake2_256(&account_nonce_key).to_vec();

        self.state
            .storage(StorageKey(storage_key), None)
            .map(|data| {
                data.map_or(0, |d| {
                    Decode::decode(&mut &d.0[..]).expect("Account nonce is valid Index")
                })
            })
            .map_err(Into::into)
    }

    /// Fetch the genesis hash
    fn fetch_genesis_hash(&self) -> impl Future<Item = Option<H256>, Error = RpcError> {
        self.chain.block_hash(Some(NumberOrHex::Number(0)))
    }

    /// Subscribe to substrate System Events
    fn subscribe_events(
        &self,
    ) -> impl Future<Item = TypedSubscriptionStream<StorageChangeSet<H256>>, Error = RpcError>
    {
        let events_key = b"System Events";
        let storage_key = twox_128(events_key);
        log::debug!("Events storage key {:?}", storage_key);

        self.state
            .subscribe_storage(Some(vec![StorageKey(storage_key.to_vec())]))
    }

    /// Submit an extrinsic, waiting for it to be finalized.
    /// If successful, returns the block hash.
    fn submit_and_watch(
        self,
        extrinsic: UncheckedExtrinsic,
    ) -> impl Future<Item = H256, Error = Error> {
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
                    .map_err(|(e,_)| e.into())
                    .and_then(|(result, _)| {
                        result
                            .ok_or(Error::from("Stream terminated"))
                            .and_then(|r| r)
                            .into_future()
                    })
            })
    }

    /// Create and submit an extrinsic and return corresponding Event if successful
    fn create_and_submit_extrinsic(
        self,
        signer: Pair,
        call: Call,
    ) -> impl Future<Item = ExtrinsicSuccess, Error = Error> {
        let account_nonce = self.fetch_nonce(&signer.public()).map_err(Into::into);
        let genesis_hash =
            self.fetch_genesis_hash()
                .map_err(Into::into)
                .and_then(|genesis_hash| {
                    future::result(genesis_hash.ok_or("Genesis hash not found".into()))
                });
        let events = self.subscribe_events().map_err(Into::into);

        account_nonce.join3(genesis_hash, events).and_then(
            move |(index, genesis_hash, events)| {
                let extrinsic =
                    create_and_sign_extrinsic(index, call, genesis_hash, &signer);
                let ext_hash =
                    H256(extrinsic.using_encoded(|encoded| blake2_256(encoded)));
                log::info!("Submitting Extrinsic `{:?}`", ext_hash);

                let chain = self.chain.clone();
                self.submit_and_watch(extrinsic)
                    .and_then(move |bh| {
                        log::info!("Fetching block {:?}", bh);
                        chain
                            .block(Some(bh))
                            .map(move |b| (bh, b))
                            .map_err(Into::into)
                    })
                    .and_then(|(h, b)| {
                        b.ok_or(format!("Failed to find block {:#x}", h).into())
                            .map(|b| (h, b))
                            .into_future()
                    })
                    .and_then(move |(bh, sb)| {
                        log::info!(
                            "Found block {:?}, with {} extrinsics",
                            bh,
                            sb.block.extrinsics.len()
                        );
                        wait_for_block_events(ext_hash, &sb, bh, events)
                    })
            },
        )
    }
}

/// Creates and signs an Extrinsic for the supplied `Call`
fn create_and_sign_extrinsic(
    index: Index,
    function: Call,
    genesis_hash: Hash,
    signer: &Pair,
) -> UncheckedExtrinsic {
    log::info!(
        "Creating Extrinsic with genesis hash {:?} and account nonce {:?}",
        genesis_hash,
        index
    );

    let extra = |nonce, fee| {
        (
            srml_system::CheckGenesis::<Runtime>::new(),
            srml_system::CheckEra::<Runtime>::from(Era::Immortal),
            srml_system::CheckNonce::<Runtime>::from(nonce),
            srml_system::CheckWeight::<Runtime>::new(),
            srml_balances::TakeFees::<Runtime>::from(fee),
        )
    };

    let raw_payload = (function, extra(index, 0), genesis_hash);
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
        extra(index, 0),
    )
}

/// Waits for events for the block triggered by the extrinsic
fn wait_for_block_events(
    ext_hash: H256,
    signed_block: &SignedBlock,
    block_hash: H256,
    events: TypedSubscriptionStream<StorageChangeSet<H256>>,
) -> impl Future<Item = ExtrinsicSuccess, Error = Error> {
    let ext_index = signed_block
        .block
        .extrinsics
        .iter()
        .position(|ext| {
            let hash = H256(ext.using_encoded(|encoded| blake2_256(encoded)));
            hash == ext_hash
        })
        .ok_or(format!("Failed to find Extrinsic with hash {:?}", ext_hash).into())
        .into_future();

    let block_hash = block_hash.clone();
    let block_events = events
        .map(|event| {
            let records = event
                .changes
                .iter()
                .filter_map(|(_key, data)| {
                    data.as_ref()
                        .and_then(|data| Decode::decode(&mut &data.0[..]))
                })
                .flat_map(|events: Vec<EventRecord>| events)
                .collect::<Vec<_>>();
            log::debug!("Block {:?}, Events {:?}", event.block, records.len());
            (event.block, records)
        })
        .filter(move |(event_block, _)| *event_block == block_hash)
        .into_future()
        .map_err(|(e, _)| e.into())
        .map(|(events, _)| events);

    block_events
        .join(ext_index)
        .map(move |(events, ext_index)| {
            let events: Vec<Event> = events
                .iter()
                .flat_map(|(_, events)| events)
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
                .collect::<Vec<Event>>();
            ExtrinsicSuccess {
                block: block_hash,
                extrinsic: ext_hash,
                events,
            }
        })
}

/// Creates, signs and submits an Extrinsic with the given `Call` to a substrate node.
pub fn submit(url: &Url, signer: Pair, call: Call) -> Result<ExtrinsicSuccess> {
    let submit = ws::connect(url.as_str())
        .expect("Url is a valid url; qed")
        .map_err(Into::into)
        .and_then(|rpc: Rpc| rpc.create_and_submit_extrinsic(signer, call));

    let mut rt = tokio::runtime::Runtime::new()?;
    rt.block_on(submit)
}
