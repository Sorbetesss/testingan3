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

use std::convert::TryInto;

use codec::{
    Decode,
    Encode,
    Error as CodecError,
};
use jsonrpsee::{
    core::{
        common::{
            Params,
            to_value as to_json_value,
        },
    },
    client::Subscription,
    Client,
};

use num_traits::bounds::Bounded;

use frame_metadata::RuntimeMetadataPrefixed;
use sp_core::{
    storage::{
        StorageChangeSet,
        StorageKey,
    },
    twox_128,
    Bytes,
};
use sp_rpc::{
    list::ListOrValue,
    number::NumberOrHex,
};
use sp_runtime::{
    generic::{
        Block,
        SignedBlock,
    },
    traits::Hash,
    OpaqueExtrinsic,
};
use sp_transaction_pool::TransactionStatus;
use sp_version::RuntimeVersion;
use std::marker::PhantomData;

use crate::{
    error::Error,
    events::{
        EventsDecoder,
        RawEvent,
        RuntimeEvent,
    },
    frame::{
        balances::Balances,
        system::{
            Phase,
            System,
            SystemEvent,
        },
    },
    metadata::Metadata,
};

pub type ChainBlock<T> = SignedBlock<Block<<T as System>::Header, OpaqueExtrinsic>>;
pub type BlockNumber<T> = NumberOrHex<<T as System>::BlockNumber>;

/// Client for substrate rpc interfaces
pub struct Rpc<T: System> {
    client: Client,
    marker: std::marker::PhantomData<T>,
}

impl<T> Rpc<T> where T: System {
    pub async fn connect_ws(url: &str) -> Result<Self, Error> {
        let raw_client = jsonrpsee::ws::ws_raw_client(&url).await?;
        Ok(Rpc { client: raw_client.into(), marker: PhantomData })
    }

    /// Fetch a storage key
    pub async fn storage<V: Decode>(
        &self,
        key: StorageKey,
    ) -> Result<Option<V>, Error> {
        // todo: update jsonrpsee::rpc_api! macro to accept shared Client (currently only RawClient)
        // until then we manually construct params here and in other methods
        let params = Params::Array(vec![to_json_value(key)?]);
        let data = self.client.request::<Option<V>>("state_getStorage", params).await?;
        match data {
            Some(data) => {
                let value = Decode::decode(&mut &data.0[..])?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Fetch the genesis hash
    pub async fn genesis_hash(&self) -> Result<T::Hash, Error> {
        let block_zero = Some(ListOrValue::Value(NumberOrHex::Number(T::BlockNumber::min_value())));
        let params = Params::Array(vec![to_json_value(block_zero)?]);
        let list_or_value = self.client.request::<ListOrValue<Option<T::Hash>>>("chain_getBlockHash", params).await?;
        match list_or_value {
            ListOrValue::Value(genesis_hash) => {
                genesis_hash.ok_or_else(|| "Genesis hash not found".into())
            }
            ListOrValue::List(_) => Err("Expected a Value, got a List".into())
        }
    }

    /// Fetch the metadata
    pub async fn metadata(&self) -> Result<Metadata, Error> {
        let bytes = self.client.request::<Bytes>("state_getMetadata", Params::None).await?;
        let meta: RuntimeMetadataPrefixed = Decode::decode(&mut &bytes[..])?;
        let metadata: Metadata = meta.try_into()?;
        Ok(metadata)
    }

    /// Get a block hash, returns hash of latest block by default
    pub async fn block_hash(
        &self,
        block_number: Option<BlockNumber<T>>,
    ) -> Result<Option<T::Hash>, Error> {
        let block_number = block_number.map(|bn| ListOrValue::Value(bn));
        let params = Params::Array(vec![to_json_value(block_number)?]);
        let list_or_value = self.client.request::<ListOrValue<Option<T::Hash>>>("chain_getBlockHash", params).await?;
        match list_or_value {
            ListOrValue::Value(hash) => Ok(hash),
            ListOrValue::List(_) => Err("Expected a Value, got a List".into()),
        }
    }

    /// Get a block hash of the latest finalized block
    pub async fn finalized_head(&self) -> Result<T::Hash, Error> {
        let hash = self.client.request::<T::Hash>("chain_getFinalizedHead", Params::None).await?;
        Ok(hash)
    }

    /// Get a Block
    pub async fn block(
        &self,
        hash: Option<T::Hash>,
    ) -> Result<Option<ChainBlock<T>>, Error> {
        let block = self.client.request::<Option<ChainBlock<T>>>("chain_getBlock", Params::None).await?;
        Ok(block)
    }

    /// Fetch the runtime version
    pub async fn runtime_version(
        &self,
        at: Option<T::Hash>,
    ) -> Result<RuntimeVersion, Error> {
        let params = Params::Array(vec![to_json_value(at)?]);
        let version = self.client.request::<RuntimeVersion>("state_getRuntimeVersion", params).await?;
        Ok(version)
    }
}

impl<T: System + Balances + 'static> Rpc<T> {
    /// Subscribe to substrate System Events
    pub async fn subscribe_events(
        &self,
    ) -> Result<Subscription<StorageChangeSet<<T as System>::Hash>>, Error>
    {
        let mut storage_key = twox_128(b"System").to_vec();
        storage_key.extend(twox_128(b"Events").to_vec());
        log::debug!("Events storage key {:?}", hex::encode(&storage_key));

        let params = Params::Array(vec![to_json_value(StorageKey(storage_key)?)]);

        let subscription = self.client.subscribe(
            "state_subscribeStorage",
            params,
            "state_unsubscribeStorage"
        ).await?;
        Ok(subscription)
    }

    /// Subscribe to blocks.
    pub async fn subscribe_blocks(
        &self,
    ) -> Result<Subscription<T::Header>, Error> {
        let subscription = self.client.subscribe(
            "chain_subscribeNewHeads",
            Params::None,
            "chain_subscribeNewHeads"
        ).await?;

        Ok(subscription)
    }

    /// Subscribe to finalized blocks.
    pub async fn subscribe_finalized_blocks(
        &self,
    ) -> Result<Subscription<T::Header>, Error> {
        let subscription = self.client.subscribe(
            "chain_subscribeFinalizedHeads",
            Params::None,
            "chain_subscribeFinalizedHeads"
        ).await?;
        Ok(subscription)
    }

    /// Create and submit an extrinsic and return corresponding Hash if successful
    pub async fn submit_extrinsic<E: Encode>(
        &self,
        extrinsic: E,
    ) -> Result<T::Hash, Error>
    {
        let bytes: Bytes = extrinsic.encode().into();
        let params = Params::Array(vec![to_json_value(bytes)?]);
        let xt_hash = self.client.request::<T::Hash>("author_submitExtrinsic", params).await?;
        Ok(xt_hash)
    }

    pub async fn watch_extrinsic<E: Encode>(&self, extrinsic: E) -> Result<Subscription<TransactionStatus<T::Hash, T::Hash>>, Error> {
        let bytes: Bytes = extrinsic.encode().into();
        let params = Params::Array(vec![to_json_value(bytes)?]);
        let subscription = self.client.subscribe(
            "author_submitAndWatchExtrinsic",
                params,
            "author_unwatchExtrinsic"
        ).await?;
        Ok(subscription)
    }

    /// Create and submit an extrinsic and return corresponding Event if successful
    pub async fn submit_and_watch_extrinsic<E: Encode + 'static>(
        self,
        extrinsic: E,
        decoder: EventsDecoder<T>,
    ) -> Result<ExtrinsicSuccess<T>, Error>
    {
        let ext_hash = T::Hashing::hash_of(&extrinsic);
        log::info!("Submitting Extrinsic `{:?}`", ext_hash);

        let mut events_sub = self.subscribe_events().await?;
        let mut xt_sub = self.watch_extrinsic(extrinsic).await?;

        let mut result: Result<ExtrinsicSuccess<T>, Error> = Err("No status received for extrinsic".into());
        while let status = xt_sub.next().await {
            log::info!("received status {:?}", status);
            match status {
                // ignore in progress extrinsic for now
                TransactionStatus::Future
                | TransactionStatus::Ready
                | TransactionStatus::Broadcast(_) => continue,
                TransactionStatus::InBlock(block_hash) => {
                    log::info!("Fetching block {:?}", block_hash);
                    let block = self.block(Some(block_hash)).await?;
                    return match block {
                        Some(signed_block) => {
                            log::info!(
                                "Found block {:?}, with {} extrinsics",
                                block_hash,
                                signed_block.block.extrinsics.len()
                            );
                            wait_for_block_events(decoder, ext_hash, signed_block, block_hash, events_sub).await
                        },
                        None => {
                            Err(format!("Failed to find block {:?}", block_hash).into())
                        }
                    }
                }
                TransactionStatus::Usurped(_) => {
                    return Err("Extrinsic Usurped".into())
                }
                TransactionStatus::Dropped => {
                    return Err("Extrinsic Dropped".into())
                }
                TransactionStatus::Invalid => {
                    return Err("Extrinsic Invalid".into())
                }
            }
        }
        return Err("No status received for extrinsic".into());
    }
}

/// Captures data for when an extrinsic is successfully included in a block
#[derive(Debug)]
pub struct ExtrinsicSuccess<T: System> {
    /// Block hash.
    pub block: T::Hash,
    /// Extrinsic hash.
    pub extrinsic: T::Hash,
    /// Raw runtime events, can be decoded by the caller.
    pub events: Vec<RuntimeEvent>,
}

impl<T: System> ExtrinsicSuccess<T> {
    /// Find the Event for the given module/variant, with raw encoded event data.
    /// Returns `None` if the Event is not found.
    pub fn find_event_raw(&self, module: &str, variant: &str) -> Option<&RawEvent> {
        self.events.iter().find_map(|evt| {
            match evt {
                RuntimeEvent::Raw(ref raw)
                    if raw.module == module && raw.variant == variant =>
                {
                    Some(raw)
                }
                _ => None,
            }
        })
    }

    /// Returns all System Events
    pub fn system_events(&self) -> Vec<&SystemEvent> {
        self.events
            .iter()
            .filter_map(|evt| {
                match evt {
                    RuntimeEvent::System(evt) => Some(evt),
                    _ => None,
                }
            })
            .collect()
    }

    /// Find the Event for the given module/variant, attempting to decode the event data.
    /// Returns `None` if the Event is not found.
    /// Returns `Err` if the data fails to decode into the supplied type
    pub fn find_event<E: Decode>(
        &self,
        module: &str,
        variant: &str,
    ) -> Option<Result<E, CodecError>> {
        self.find_event_raw(module, variant)
            .map(|evt| E::decode(&mut &evt.data[..]))
    }
}

/// Waits for events for the block triggered by the extrinsic
pub async fn wait_for_block_events<T: System + Balances + 'static>(
    decoder: EventsDecoder<T>,
    ext_hash: T::Hash,
    signed_block: ChainBlock<T>,
    block_hash: T::Hash,
    events_subscription: Subscription<StorageChangeSet<T::Hash>>,
) -> Result<ExtrinsicSuccess<T>, Error> {
    let ext_index = signed_block
        .block
        .extrinsics
        .iter()
        .position(|ext| {
            let hash = T::Hashing::hash_of(ext);
            hash == ext_hash
        })
        .ok_or_else(|| {
            format!("Failed to find Extrinsic with hash {:?}", ext_hash).into()
        })?;

    let mut subscription = events_subscription;
    while let change_set = subscription.next().await {
        // only interested in events for the given block
        if change_set.block != block_hash {
            continue
        }
        let events = match change_set {
            None => Vec::new(),
            Some(change_set) => {
                let mut events = Vec::new();
                for (_key, data) in change_set.changes {
                    if let Some(data) = data {
                        match decoder.decode_events(&mut &data.0[..]) {
                            Ok(raw_events) => {
                                for (phase, event) in raw_events {
                                    if let Phase::ApplyExtrinsic(i) = phase {
                                        if i as usize == ext_index {
                                            events.push(event)
                                        }
                                    }
                                }
                            }
                            Err(err) => return Err(err.into()),
                        }
                    }
                }
                events
            }
        };
        return Ok(ExtrinsicSuccess {
            block: block_hash,
            extrinsic: ext_hash,
            events,
        })
    }
    return Err(format!("No events found for block {}", block_hash).into())
}
