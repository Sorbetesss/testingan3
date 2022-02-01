// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is part of subxt.
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
// along with subxt.  If not, see <http://www.gnu.org/licenses/>.

use crate::{
    error::BasicError,
    events::{
        EventsDecoder,
        RawEvent,
    },
    rpc::Rpc,
    Config,
    Event,
    Phase,
};
use jsonrpsee::core::{
    client::Subscription,
    DeserializeOwned,
};
use sp_core::{
    storage::{
        StorageChangeSet,
        StorageKey,
    },
    twox_128,
};
use sp_runtime::traits::Header;
use std::collections::VecDeque;

/// Event subscription simplifies filtering a storage change set stream for
/// events of interest.
pub struct EventSubscription<'a, T: Config> {
    block_reader: BlockReader<'a, T>,
    block: Option<T::Hash>,
    extrinsic: Option<usize>,
    event: Option<(&'static str, &'static str)>,
    events: VecDeque<(<T as Config>::Hash, usize, RawEvent)>,
    finished: bool,
}

enum BlockReader<'a, T: Config> {
    Decoder {
        subscription: EventStorageSubscription<T>,
        decoder: &'a EventsDecoder<T>,
    },
    /// Mock event listener for unit tests
    #[cfg(test)]
    Mock(Box<dyn Iterator<Item = (T::Hash, Result<Vec<(Phase, usize, RawEvent)>, BasicError>)>>),
}

impl<'a, T: Config> BlockReader<'a, T> {
    async fn next(
        &mut self,
    ) -> Option<(T::Hash, Result<Vec<(Phase, usize, RawEvent)>, BasicError>)> {
        match self {
            BlockReader::Decoder {
                subscription,
                decoder,
            } => {
                let change_set = subscription.next().await?;
                let events: Result<Vec<_>, _> = change_set
                    .changes
                    .into_iter()
                    .filter_map(|(_key, change)| {
                        Some(decoder.decode_events(&mut change?.0.as_slice()))
                    })
                    .collect();

                let flattened_events =
                    events.map(|x| {
                        x.into_iter()
                            .flatten()
                            .enumerate()
                            .map(|(idx, (phase, raw))| {
                                (phase, idx, raw)
                            })
                            .collect()
                    });
                Some((change_set.block, flattened_events))
            }
            #[cfg(test)]
            BlockReader::Mock(it) => it.next(),
        }
    }
}

impl<'a, T: Config> EventSubscription<'a, T> {
    /// Creates a new event subscription.
    pub fn new(
        subscription: EventStorageSubscription<T>,
        decoder: &'a EventsDecoder<T>,
    ) -> Self {
        Self {
            block_reader: BlockReader::Decoder {
                subscription,
                decoder,
            },
            block: None,
            extrinsic: None,
            event: None,
            events: Default::default(),
            finished: false,
        }
    }

    /// Only returns events contained in the block with the given hash.
    pub fn filter_block(&mut self, block: T::Hash) {
        self.block = Some(block);
    }

    /// Only returns events from block emitted by extrinsic with index.
    pub fn filter_extrinsic(&mut self, block: T::Hash, ext_index: usize) {
        self.block = Some(block);
        self.extrinsic = Some(ext_index);
    }

    /// Filters events by type.
    pub fn filter_event<Ev: Event>(&mut self) {
        self.event = Some((Ev::PALLET, Ev::EVENT));
    }

    /// Gets the next event.
    pub async fn next(&mut self) -> Option<Result<RawEvent, BasicError>> {
        self.next_context()
            .await
            .map(|res| res.map(|(_, _, event)| event))
    }
    /// Gets the next event with the associated block hash and its corresponding
    /// event index.
    pub async fn next_context(
        &mut self,
    ) -> Option<Result<(<T as Config>::Hash, usize, RawEvent), BasicError>> {
        loop {
            if let Some(raw_event) = self.events.pop_front() {
                return Some(Ok(raw_event))
            }
            if self.finished {
                return None
            }
            // always return None if subscription has closed
            let (received_hash, events) = self.block_reader.next().await?;
            if let Some(hash) = self.block.as_ref() {
                if &received_hash == hash {
                    self.finished = true;
                } else {
                    continue
                }
            }

            match events {
                Err(err) => return Some(Err(err)),
                Ok(raw_events) => {
                    for (phase, idx, raw) in raw_events {
                        if let Some(ext_index) = self.extrinsic {
                            if !matches!(phase, Phase::ApplyExtrinsic(i) if i as usize == ext_index)
                            {
                                continue
                            }
                        }
                        if let Some((module, variant)) = self.event {
                            if raw.pallet != module || raw.variant != variant {
                                continue
                            }
                        }
                        self.events.push_back((received_hash, idx, raw));
                    }
                }
            }
        }
    }
}

pub(crate) struct SystemEvents(StorageKey);

impl SystemEvents {
    pub(crate) fn new() -> Self {
        let mut storage_key = twox_128(b"System").to_vec();
        storage_key.extend(twox_128(b"Events").to_vec());
        log::debug!("Events storage key {:?}", hex::encode(&storage_key));
        Self(StorageKey(storage_key))
    }
}

impl From<SystemEvents> for StorageKey {
    fn from(key: SystemEvents) -> Self {
        key.0
    }
}

/// Event subscription to only fetch finalized storage changes.
pub struct FinalizedEventStorageSubscription<T: Config> {
    rpc: Rpc<T>,
    subscription: Subscription<T::Header>,
    storage_changes: VecDeque<StorageChangeSet<T::Hash>>,
    storage_key: StorageKey,
}

impl<T: Config> FinalizedEventStorageSubscription<T> {
    /// Creates a new finalized event storage subscription.
    pub fn new(rpc: Rpc<T>, subscription: Subscription<T::Header>) -> Self {
        Self {
            rpc,
            subscription,
            storage_changes: Default::default(),
            storage_key: SystemEvents::new().into(),
        }
    }

    /// Gets the next change_set.
    pub async fn next(&mut self) -> Option<StorageChangeSet<T::Hash>> {
        loop {
            if let Some(storage_change) = self.storage_changes.pop_front() {
                return Some(storage_change)
            }
            let header: T::Header =
                read_subscription_response("HeaderSubscription", &mut self.subscription)
                    .await?;
            self.storage_changes.extend(
                self.rpc
                    .query_storage_at(&[self.storage_key.clone()], Some(header.hash()))
                    .await
                    .ok()?,
            );
        }
    }
}

/// Wrapper over imported and finalized event subscriptions.
pub enum EventStorageSubscription<T: Config> {
    /// Events that are InBlock
    Imported(Subscription<StorageChangeSet<T::Hash>>),
    /// Events that are Finalized
    Finalized(FinalizedEventStorageSubscription<T>),
}

impl<T: Config> EventStorageSubscription<T> {
    /// Gets the next change_set from the subscription.
    pub async fn next(&mut self) -> Option<StorageChangeSet<T::Hash>> {
        match self {
            Self::Imported(event_sub) => {
                read_subscription_response("StorageChangeSetSubscription", event_sub)
                    .await
            }
            Self::Finalized(event_sub) => event_sub.next().await,
        }
    }
}

async fn read_subscription_response<T>(
    sub_name: &str,
    sub: &mut Subscription<T>,
) -> Option<T>
where
    T: DeserializeOwned,
{
    match sub.next().await {
        Some(Ok(next)) => Some(next),
        Some(Err(e)) => {
            log::error!("Subscription {} failed: {:?} dropping", sub_name, e);
            None
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultConfig;
    use sp_core::H256;

    fn named_event(event_name: &str) -> RawEvent {
        RawEvent {
            data: sp_core::Bytes::from(Vec::new()),
            pallet: event_name.to_string(),
            variant: event_name.to_string(),
            pallet_index: 0,
            variant_index: 0,
        }
    }

    #[async_std::test]
    /// test that filters work correctly, and are independent of each other
    async fn test_filters() {
        let mut events: Vec<(H256, Phase, usize, RawEvent)> = vec![];
        // create all events
        for block_hash in [H256::from([0; 32]), H256::from([1; 32])] {
            for phase in [
                Phase::Initialization,
                Phase::ApplyExtrinsic(0),
                Phase::ApplyExtrinsic(1),
                Phase::Finalization,
            ] {
                [named_event("a"), named_event("b")]
                    .iter()
                    .enumerate()
                    .for_each(|(idx, event)| {
                        events.push((
                            block_hash,
                            phase.clone(),
                            // The event index
                            idx,
                            {
                                // set variant index so we can uniquely identify
                                // the event, independently from the event index
                                let mut event = event.clone();
                                event.variant_index = (idx * 2) as u8;
                                event
                            }
                        ))
                    });
            }
        }

        let half_len = events.len() / 2;

        for block_filter in [None, Some(H256::from([1; 32]))] {
            for extrinsic_filter in [None, Some(1)] {
                for event_filter in [None, Some(("b", "b"))] {
                    let mut subscription: EventSubscription<DefaultConfig> =
                        EventSubscription {
                            block_reader: BlockReader::Mock(Box::new(
                                vec![
                                    (
                                        events[0].0,
                                        Ok(events
                                            .iter()
                                            .take(half_len)
                                            .map(|(_, phase, idx, event)| {
                                                (phase.clone(), *idx, event.clone())
                                            })
                                            .collect()),
                                    ),
                                    (
                                        events[half_len].0,
                                        Ok(events
                                            .iter()
                                            .skip(half_len)
                                            .map(|(_, phase, idx, event)| {
                                                (phase.clone(), *idx, event.clone())
                                            })
                                            .collect()),
                                    ),
                                ]
                                .into_iter(),
                            )),
                            block: block_filter,
                            extrinsic: extrinsic_filter,
                            event: event_filter,
                            events: Default::default(),
                            finished: false,
                        };

                    let mut expected_events: Vec<(H256, Phase, usize, RawEvent)> = events.clone();

                    if let Some(hash) = block_filter {
                        expected_events.retain(|(h, _, _, _)| h == &hash);
                    }
                    if let Some(idx) = extrinsic_filter {
                        expected_events.retain(|(_, phase, _,  _)| matches!(phase, Phase::ApplyExtrinsic(i) if *i as usize == idx));
                    }
                    if let Some(name) = event_filter {
                        expected_events.retain(|(_, _, _, event)| event.pallet == name.0);
                    }

                    for expected_event in expected_events {
                        assert_eq!(
                            subscription.next().await.unwrap().unwrap(),
                            expected_event.3
                        );
                    }
                    assert!(subscription.next().await.is_none());
                }
            }
        }
    }

    #[async_std::test]
    async fn test_context() {
        let mut events: Vec<(H256, Phase, usize, RawEvent)> = vec![];
        // create all events
        for block_hash in [H256::from([0; 32]), H256::from([1; 32])] {
            for phase in [
                Phase::Initialization,
                Phase::ApplyExtrinsic(0),
                Phase::ApplyExtrinsic(1),
                Phase::Finalization,
            ] {
                [named_event("a"), named_event("b")]
                    .iter()
                    .enumerate()
                    .for_each(|(idx, event)| {
                        events.push((
                            block_hash,
                            phase.clone(),
                            // The event index
                            idx,
                            {
                                // set variant index so we can uniquely identify
                                // the event, independently from the event index
                                let mut event = event.clone();
                                event.variant_index = (idx * 2) as u8;
                                event
                            }
                        ))
                    });
            }
        }

        let half_len = events.len() / 2;

        let mut subscription: EventSubscription<DefaultConfig> =
            EventSubscription {
                block_reader: BlockReader::Mock(Box::new(
                    vec![
                        (
                            events[0].0,
                            Ok(events
                                .iter()
                                .take(half_len)
                                .map(|(_, phase, idx, event)| {
                                    (phase.clone(), *idx, event.clone())
                                })
                                .collect()),
                        ),
                        (
                            events[half_len].0,
                            Ok(events
                                .iter()
                                .skip(half_len)
                                .map(|(_, phase, idx, event)| {
                                    (phase.clone(), *idx, event.clone())
                                })
                                .collect()),
                        ),
                    ]
                    .into_iter(),
                )),
                block: None,
                extrinsic: None,
                event: None,
                events: Default::default(),
                finished: false,
            };

        let expected_events: Vec<(H256, Phase, usize, RawEvent)> = events.clone();

        for exp in expected_events {
            assert_eq!(
                subscription.next_context().await.unwrap().unwrap(),
                // (block_hash, event_idx, event)
                (exp.0, exp.2, exp.3)
            );
        }
        assert!(subscription.next().await.is_none());
    }

}
