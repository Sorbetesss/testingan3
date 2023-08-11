// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use super::storage_address::{StorageAddress, Yes};

use crate::{
    client::OnlineClientT,
    error::{Error, MetadataError},
    metadata::{DecodeWithMetadata, Metadata},
    rpc::types::{StorageData, StorageKey},
    Config,
};
use codec::Decode;
use derivative::Derivative;
use std::{future::Future, marker::PhantomData};
use subxt_metadata::{PalletMetadata, StorageEntryMetadata, StorageEntryType};

/// Query the runtime storage.
#[derive(Derivative)]
#[derivative(Clone(bound = "Client: Clone"))]
pub struct Storage<T: Config, Client> {
    client: Client,
    block_hash: T::Hash,
    _marker: PhantomData<T>,
}

impl<T: Config, Client> Storage<T, Client> {
    /// Create a new [`Storage`]
    pub(crate) fn new(client: Client, block_hash: T::Hash) -> Self {
        Self {
            client,
            block_hash,
            _marker: PhantomData,
        }
    }
}

impl<T, Client> Storage<T, Client>
where
    T: Config,
    Client: OnlineClientT<T>,
{
    /// Fetch the raw encoded value at the address/key given.
    pub fn fetch_raw<'address>(
        &self,
        key: &'address [u8],
    ) -> impl Future<Output = Result<Option<Vec<u8>>, Error>> + 'address {
        let client = self.client.clone();
        let block_hash = self.block_hash;
        // Ensure that the returned future doesn't have a lifetime tied to api.storage(),
        // which is a temporary thing we'll be throwing away quickly:
        async move {
            let data = client.rpc().storage(key, Some(block_hash)).await?;
            Ok(data.map(|d| d.0))
        }
    }

    /// Fetch a decoded value from storage at a given address.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use subxt::{ PolkadotConfig, OnlineClient };
    ///
    /// #[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_full.scale")]
    /// pub mod polkadot {}
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let api = OnlineClient::<PolkadotConfig>::new().await.unwrap();
    ///
    /// // Address to a storage entry we'd like to access.
    /// let address = polkadot::storage().xcm_pallet().queries(&12345);
    ///
    /// // Fetch just the keys, returning up to 10 keys.
    /// let value = api
    ///     .storage()
    ///     .at_latest()
    ///     .await
    ///     .unwrap()
    ///     .fetch(&address)
    ///     .await
    ///     .unwrap();
    ///
    /// println!("Value: {:?}", value);
    /// # }
    /// ```
    pub fn fetch<'address, Address>(
        &self,
        address: &'address Address,
    ) -> impl Future<Output = Result<Option<Address::Target>, Error>> + 'address
    where
        Address: StorageAddress<IsFetchable = Yes> + 'address,
    {
        let client = self.clone();
        async move {
            let metadata = client.client.metadata();
            let (pallet, entry) =
                lookup_entry_details(address.pallet_name(), address.entry_name(), &metadata)?;

            // Metadata validation checks whether the static address given
            // is likely to actually correspond to a real storage entry or not.
            // if not, it means static codegen doesn't line up with runtime
            // metadata.
            validate_storage_address(address, pallet)?;

            // Look up the return type ID to enable DecodeWithMetadata:
            let lookup_bytes = super::utils::storage_address_bytes(address, &metadata)?;
            if let Some(data) = client.fetch_raw(&lookup_bytes).await? {
                let val =
                    decode_storage_with_metadata::<Address::Target>(&mut &*data, &metadata, entry)?;
                Ok(Some(val))
            } else {
                Ok(None)
            }
        }
    }

    /// Fetch a StorageKey that has a default value with an optional block hash.
    pub fn fetch_or_default<'address, Address>(
        &self,
        address: &'address Address,
    ) -> impl Future<Output = Result<Address::Target, Error>> + 'address
    where
        Address: StorageAddress<IsFetchable = Yes, IsDefaultable = Yes> + 'address,
    {
        let client = self.clone();
        async move {
            let pallet_name = address.pallet_name();
            let entry_name = address.entry_name();
            // Metadata validation happens via .fetch():
            if let Some(data) = client.fetch(address).await? {
                Ok(data)
            } else {
                let metadata = client.client.metadata();
                let (_pallet_metadata, storage_entry) =
                    lookup_entry_details(pallet_name, entry_name, &metadata)?;

                let return_ty_id = return_type_from_storage_entry_type(storage_entry.entry_type());
                let bytes = &mut storage_entry.default_bytes();

                let val = Address::Target::decode_with_metadata(bytes, return_ty_id, &metadata)?;
                Ok(val)
            }
        }
    }

    /// Fetch up to `count` keys for a storage map in lexicographic order.
    ///
    /// Supports pagination by passing a value to `start_key`.
    pub fn fetch_keys<'address>(
        &self,
        key: &'address [u8],
        count: u32,
        start_key: Option<&'address [u8]>,
    ) -> impl Future<Output = Result<Vec<StorageKey>, Error>> + 'address {
        let client = self.client.clone();
        let block_hash = self.block_hash;
        async move {
            let keys = client
                .rpc()
                .storage_keys_paged(key, count, start_key, Some(block_hash))
                .await?;
            Ok(keys)
        }
    }

    /// Returns an iterator of key value pairs.
    ///
    /// ```no_run
    /// use subxt::{ PolkadotConfig, OnlineClient };
    ///
    /// #[subxt::subxt(runtime_metadata_path = "../artifacts/polkadot_metadata_full.scale")]
    /// pub mod polkadot {}
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let api = OnlineClient::<PolkadotConfig>::new().await.unwrap();
    ///
    /// // Address to the root of a storage entry that we'd like to iterate over.
    /// let address = polkadot::storage().xcm_pallet().version_notifiers_iter();
    ///
    /// // Iterate over keys and values at that address.
    /// let mut iter = api
    ///     .storage()
    ///     .at_latest()
    ///     .await
    ///     .unwrap()
    ///     .iter(address, 10)
    ///     .await
    ///     .unwrap();
    ///
    /// while let Some((key, value)) = iter.next().await.unwrap() {
    ///     println!("Key: 0x{}", hex::encode(&key));
    ///     println!("Value: {}", value);
    /// }
    /// # }
    /// ```
    pub fn iter<Address>(
        &self,
        address: Address,
        page_size: u32,
    ) -> impl Future<Output = Result<KeyIter<T, Client, Address::Target>, Error>> + 'static
    where
        Address: StorageAddress<IsIterable = Yes> + 'static,
    {
        let client = self.clone();
        let block_hash = self.block_hash;
        async move {
            let metadata = client.client.metadata();
            let (pallet, entry) =
                lookup_entry_details(address.pallet_name(), address.entry_name(), &metadata)?;

            // Metadata validation checks whether the static address given
            // is likely to actually correspond to a real storage entry or not.
            // if not, it means static codegen doesn't line up with runtime
            // metadata.
            validate_storage_address(&address, pallet)?;

            // Look up the return type for flexible decoding. Do this once here to avoid
            // potentially doing it every iteration if we used `decode_storage_with_metadata`
            // in the iterator.
            let return_type_id = return_type_from_storage_entry_type(entry.entry_type());

            // The root pallet/entry bytes for this storage entry:
            let address_root_bytes = super::utils::storage_address_root_bytes(&address);

            Ok(KeyIter {
                client,
                address_root_bytes,
                metadata,
                return_type_id,
                block_hash,
                count: page_size,
                start_key: None,
                buffer: Default::default(),
                _marker: std::marker::PhantomData,
            })
        }
    }

    /// The storage version of a pallet.
    /// The storage version refers to the `frame_support::traits::Metadata::StorageVersion` type.
    pub async fn storage_version(&self, pallet_name: impl AsRef<str>) -> Result<u16, Error> {
        // check that the pallet exists in the metadata:
        self.client
            .metadata()
            .pallet_by_name(pallet_name.as_ref())
            .ok_or_else(|| MetadataError::PalletNameNotFound(pallet_name.as_ref().into()))?;

        // construct the storage key. This is done similarly in `frame_support::traits::metadata::StorageVersion::storage_key()`.
        pub const STORAGE_VERSION_STORAGE_KEY_POSTFIX: &[u8] = b":__STORAGE_VERSION__:";
        let mut key_bytes: Vec<u8> = vec![];
        key_bytes.extend(&sp_core_hashing::twox_128(pallet_name.as_ref().as_bytes()));
        key_bytes.extend(&sp_core_hashing::twox_128(
            STORAGE_VERSION_STORAGE_KEY_POSTFIX,
        ));

        // fetch the raw bytes and decode them into the StorageVersion struct:
        let storage_version_bytes = self.fetch_raw(&key_bytes).await?.ok_or_else(|| {
            format!(
                "Unexpected: entry for storage version in pallet \"{}\" not found",
                pallet_name.as_ref()
            )
        })?;
        u16::decode(&mut &storage_version_bytes[..]).map_err(Into::into)
    }

    /// Fetches the Wasm code of the runtime.
    pub async fn runtime_wasm_code(&self) -> Result<Vec<u8>, Error> {
        // note: this should match the `CODE` constant in `sp_core::storage::well_known_keys`
        const CODE: &str = ":code";
        self.fetch_raw(CODE.as_bytes()).await?.ok_or_else(|| {
            format!("Unexpected: entry for well known key \"{CODE}\" not found").into()
        })
    }
}

/// Iterates over key value pairs in a map.
pub struct KeyIter<T: Config, Client, ReturnTy> {
    client: Storage<T, Client>,
    address_root_bytes: Vec<u8>,
    return_type_id: u32,
    metadata: Metadata,
    count: u32,
    block_hash: T::Hash,
    start_key: Option<StorageKey>,
    buffer: Vec<(StorageKey, StorageData)>,
    _marker: std::marker::PhantomData<ReturnTy>,
}

impl<'a, T, Client, ReturnTy> KeyIter<T, Client, ReturnTy>
where
    T: Config,
    Client: OnlineClientT<T>,
    ReturnTy: DecodeWithMetadata,
{
    /// Returns the next key value pair from a map.
    pub async fn next(&mut self) -> Result<Option<(StorageKey, ReturnTy)>, Error> {
        loop {
            if let Some((k, v)) = self.buffer.pop() {
                let val = ReturnTy::decode_with_metadata(
                    &mut &v.0[..],
                    self.return_type_id,
                    &self.metadata,
                )?;
                return Ok(Some((k, val)));
            } else {
                let start_key = self.start_key.take();
                let keys = self
                    .client
                    .fetch_keys(
                        &self.address_root_bytes,
                        self.count,
                        start_key.as_ref().map(|k| &*k.0),
                    )
                    .await?;

                if keys.is_empty() {
                    return Ok(None);
                }

                self.start_key = keys.last().cloned();

                let change_sets = self
                    .client
                    .client
                    .rpc()
                    .query_storage_at(keys.iter().map(|k| &*k.0), Some(self.block_hash))
                    .await?;
                for change_set in change_sets {
                    for (k, v) in change_set.changes {
                        if let Some(v) = v {
                            self.buffer.push((k, v));
                        }
                    }
                }
                debug_assert_eq!(self.buffer.len(), keys.len());
            }
        }
    }
}

/// Validate a storage address against the metadata.
pub(crate) fn validate_storage_address<Address: StorageAddress>(
    address: &Address,
    pallet: PalletMetadata<'_>,
) -> Result<(), Error> {
    if let Some(hash) = address.validation_hash() {
        validate_storage(pallet, address.entry_name(), hash)?;
    }
    Ok(())
}

/// Return details about the given storage entry.
fn lookup_entry_details<'a>(
    pallet_name: &str,
    entry_name: &str,
    metadata: &'a Metadata,
) -> Result<(PalletMetadata<'a>, &'a StorageEntryMetadata), Error> {
    let pallet_metadata = metadata.pallet_by_name_err(pallet_name)?;
    let storage_metadata = pallet_metadata
        .storage()
        .ok_or_else(|| MetadataError::StorageNotFoundInPallet(pallet_name.to_owned()))?;
    let storage_entry = storage_metadata
        .entry_by_name(entry_name)
        .ok_or_else(|| MetadataError::StorageEntryNotFound(entry_name.to_owned()))?;
    Ok((pallet_metadata, storage_entry))
}

/// Validate a storage entry against the metadata.
fn validate_storage(
    pallet: PalletMetadata<'_>,
    storage_name: &str,
    hash: [u8; 32],
) -> Result<(), Error> {
    let Some(expected_hash) = pallet.storage_hash(storage_name) else {
        return Err(MetadataError::IncompatibleCodegen.into());
    };
    if expected_hash != hash {
        return Err(MetadataError::IncompatibleCodegen.into());
    }
    Ok(())
}

/// Fetch the return type out of a [`StorageEntryType`].
fn return_type_from_storage_entry_type(entry: &StorageEntryType) -> u32 {
    match entry {
        StorageEntryType::Plain(ty) => *ty,
        StorageEntryType::Map { value_ty, .. } => *value_ty,
    }
}

/// Given some bytes, a pallet and storage name, decode the response.
fn decode_storage_with_metadata<T: DecodeWithMetadata>(
    bytes: &mut &[u8],
    metadata: &Metadata,
    storage_metadata: &StorageEntryMetadata,
) -> Result<T, Error> {
    let ty = storage_metadata.entry_type();
    let return_ty = return_type_from_storage_entry_type(ty);
    let val = T::decode_with_metadata(bytes, return_ty, metadata)?;
    Ok(val)
}
