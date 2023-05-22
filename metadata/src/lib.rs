// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

mod from_into;
mod utils;

use scale_info::{form::PortableForm, PortableRegistry, TypeDef, Variant};
use std::collections::HashMap;
use std::sync::Arc;
use utils::ordered_map::OrderedMap;
use utils::variant_index::VariantIndex;

type ArcStr = Arc<str>;

pub use utils::validation::MetadataHasher;

/// Node metadata. This can be constructed by providing some compatible [`frame_metadata`]
/// which is then decoded into this. We aim to preserve all of the existing information in
/// the incoming metadata while optimizing the format a little for Subxt's use cases.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Type registry containing all types used in the metadata.
    types: PortableRegistry,
    /// Metadata of all the pallets.
    pallets: OrderedMap<ArcStr, PalletMetadataInner>,
    /// Find the location in the pallet Vec by pallet index.
    pallets_by_index: HashMap<u8, usize>,
    /// Metadata of the extrinsic.
    extrinsic: ExtrinsicMetadata,
    /// The type ID of the `Runtime` type.
    runtime_ty: u32,
    /// The type Id of the `DispatchError` type, which Subxt makes use of.
    dispatch_error_ty: u32,
    /// Details about each of the runtime API traits.
    apis: OrderedMap<ArcStr, RuntimeApiMetadataInner>,
}

impl Metadata {
    /// Access the underlying type registry.
    pub fn types(&self) -> &PortableRegistry {
        &self.types
    }

    /// The type ID of the `Runtime` type.
    pub fn runtime_ty(&self) -> u32 {
        self.runtime_ty
    }

    /// The type ID of the `DispatchError` type.
    pub fn dispatch_error_ty(&self) -> u32 {
        self.dispatch_error_ty
    }

    /// Return details about the extrinsic format.
    pub fn extrinsic(&self) -> &ExtrinsicMetadata {
        &self.extrinsic
    }

    /// An iterator over all of the available pallets.
    pub fn pallets(&self) -> impl Iterator<Item = PalletMetadata<'_>> {
        self.pallets.values().iter().map(|inner| PalletMetadata {
            inner,
            types: self.types(),
        })
    }

    /// Access a pallet given its encoded variant index.
    pub fn pallet_by_index(&self, variant_index: u8) -> Option<PalletMetadata<'_>> {
        let inner = self
            .pallets_by_index
            .get(&variant_index)
            .and_then(|i| self.pallets.get_by_index(*i))?;

        Some(PalletMetadata {
            inner,
            types: self.types(),
        })
    }

    /// Access a pallet given its name.
    pub fn pallet_by_name(&self, pallet_name: &str) -> Option<PalletMetadata<'_>> {
        let inner = self.pallets.get_by_key(pallet_name)?;

        Some(PalletMetadata {
            inner,
            types: self.types(),
        })
    }

    /// An iterator over all of the runtime APIs.
    pub fn runtime_api_traits(&self) -> impl Iterator<Item = RuntimeApiMetadata<'_>> {
        self.apis.values().iter().map(|inner| RuntimeApiMetadata {
            inner,
            types: self.types(),
        })
    }

    /// Access a runtime API trait given its name.
    pub fn runtime_api_trait_by_name(&'_ self, name: &str) -> Option<RuntimeApiMetadata<'_>> {
        let inner = self.apis.get_by_key(name)?;
        Some(RuntimeApiMetadata {
            inner,
            types: self.types(),
        })
    }

    /// Obtain a unique hash representing this metadata or specific parts of it.
    pub fn hasher(&self) -> MetadataHasher {
        MetadataHasher::new(self)
    }

    /// Filter out any pallets that we don't want to keep, retaining only those that we do.
    pub fn retain<F, G>(&mut self, pallet_filter: F, api_filter: G)
    where
        F: FnMut(&str) -> bool,
        G: FnMut(&str) -> bool,
    {
        utils::retain::retain_metadata(self, pallet_filter, api_filter);
    }
}

/// Metadata for a specific pallet.
#[derive(Debug, Clone, Copy)]
pub struct PalletMetadata<'a> {
    inner: &'a PalletMetadataInner,
    types: &'a PortableRegistry,
}

impl<'a> PalletMetadata<'a> {
    /// The pallet name.
    pub fn name(&self) -> &'a str {
        &self.inner.name
    }

    /// The pallet index.
    pub fn index(&self) -> u8 {
        self.inner.index
    }

    /// The pallet docs.
    pub fn docs(&self) -> &'a [String] {
        &self.inner.docs
    }

    /// Type ID for the pallet's Call type, if it exists.
    pub fn call_ty_id(&self) -> Option<u32> {
        self.inner.call_ty
    }

    /// Type ID for the pallet's Event type, if it exists.
    pub fn event_ty_id(&self) -> Option<u32> {
        self.inner.event_ty
    }

    /// Type ID for the pallet's Error type, if it exists.
    pub fn error_ty_id(&self) -> Option<u32> {
        self.inner.error_ty
    }

    /// Return metadata about the pallet's storage entries.
    pub fn storage(&self) -> Option<&'a StorageMetadata> {
        self.inner.storage.as_ref()
    }

    /// Return all of the event variants, if an event type exists.
    pub fn event_variants(&self) -> Option<&'a [Variant<PortableForm>]> {
        self.variants(self.inner.event_ty?)
    }

    /// Return an event variant given it's encoded variant index.
    pub fn event_variant_by_index(&self, variant_index: u8) -> Option<&'a Variant<PortableForm>> {
        self.inner.event_variant_index.lookup_by_index(
            variant_index,
            self.inner.event_ty,
            self.types,
        )
    }

    /// Return all of the call variants, if a call type exists.
    pub fn call_variants(&self) -> Option<&'a [Variant<PortableForm>]> {
        self.variants(self.inner.call_ty?)
    }

    /// Return a call variant given it's encoded variant index.
    pub fn call_variant_by_index(&self, variant_index: u8) -> Option<&'a Variant<PortableForm>> {
        self.inner
            .call_variant_index
            .lookup_by_index(variant_index, self.inner.call_ty, self.types)
    }

    /// Return a call variant given it's name.
    pub fn call_variant_by_name(&self, call_name: &str) -> Option<&'a Variant<PortableForm>> {
        self.inner
            .call_variant_index
            .lookup_by_name(call_name, self.inner.call_ty, self.types)
    }

    /// Return all of the error variants, if an error type exists.
    pub fn error_variants(&self) -> Option<&'a [Variant<PortableForm>]> {
        self.variants(self.inner.error_ty?)
    }

    /// Return an error variant given it's encoded variant index.
    pub fn error_variant_by_index(&self, variant_index: u8) -> Option<&'a Variant<PortableForm>> {
        self.inner.error_variant_index.lookup_by_index(
            variant_index,
            self.inner.error_ty,
            self.types,
        )
    }

    /// Return constant details given the constant name.
    pub fn constant_by_name(&self, name: &str) -> Option<&'a ConstantMetadata> {
        self.inner.constants.get_by_key(name)
    }

    /// An iterator over the constants in this pallet.
    pub fn constants(&self) -> impl Iterator<Item = &'a ConstantMetadata> {
        self.inner.constants.values().iter()
    }

    /// Return a hash for the storage entry, or None if it was not found.
    pub fn storage_hash(&self, entry_name: &str) -> Option<[u8; 32]> {
        crate::utils::validation::get_storage_hash(self, entry_name)
    }

    /// Return a hash for the constant, or None if it was not found.
    pub fn constant_hash(&self, constant_name: &str) -> Option<[u8; 32]> {
        crate::utils::validation::get_constant_hash(self, constant_name)
    }

    /// Return a hash for the call, or None if it was not found.
    pub fn call_hash(&self, call_name: &str) -> Option<[u8; 32]> {
        crate::utils::validation::get_call_hash(self, call_name)
    }

    fn variants(&self, variant_type_id: u32) -> Option<&'a [Variant<PortableForm>]> {
        let TypeDef::Variant(v) = &self.types.resolve(variant_type_id)?.type_def else {
            return None;
        };
        Some(&v.variants)
    }
}

#[derive(Debug, Clone)]
struct PalletMetadataInner {
    /// Pallet name.
    name: ArcStr,
    /// Pallet index.
    index: u8,
    /// Pallet storage metadata.
    storage: Option<StorageMetadata>,
    /// Type ID for the pallet Call enum.
    call_ty: Option<u32>,
    /// Call variants by name/u8.
    call_variant_index: VariantIndex,
    /// Type ID for the pallet Event enum.
    event_ty: Option<u32>,
    /// Event variants by name/u8.
    event_variant_index: VariantIndex,
    /// Type ID for the pallet Error enum.
    error_ty: Option<u32>,
    /// Error variants by name/u8.
    error_variant_index: VariantIndex,
    /// Map from constant name to constant details.
    constants: OrderedMap<ArcStr, ConstantMetadata>,
    /// Pallet documentation.
    docs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StorageMetadata {
    /// The common prefix used by all storage entries.
    prefix: String,
    /// Map from storage entry name to details.
    entries: OrderedMap<ArcStr, StorageEntryMetadata>,
}

impl StorageMetadata {
    /// The common prefix used by all storage entries.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// An iterator over the storage entries.
    pub fn entries(&self) -> impl Iterator<Item = &StorageEntryMetadata> {
        self.entries.values().iter()
    }

    /// Return a specific storage entry given its name.
    pub fn entry_by_name(&self, name: &str) -> Option<&StorageEntryMetadata> {
        self.entries.get_by_key(name)
    }
}

#[derive(Debug, Clone)]
pub struct StorageEntryMetadata {
    /// Variable name of the storage entry.
    name: ArcStr,
    /// An `Option` modifier of that storage entry.
    modifier: StorageEntryModifier,
    /// Type of the value stored in the entry.
    entry_type: StorageEntryType,
    /// Default value (SCALE encoded).
    default: Vec<u8>,
    /// Storage entry documentation.
    docs: Vec<String>,
}

impl StorageEntryMetadata {
    /// Name of this entry.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Is the entry value optional or does it have a default value.
    pub fn modifier(&self) -> StorageEntryModifier {
        self.modifier
    }
    /// Type of the storage entry.
    pub fn entry_type(&self) -> &StorageEntryType {
        &self.entry_type
    }
    /// The SCALE encoded default value for this entry.
    pub fn default_bytes(&self) -> &[u8] {
        &self.default
    }
    /// Storage entry documentation.
    pub fn docs(&self) -> &[String] {
        &self.docs
    }
}

#[derive(Debug, Clone)]
pub enum StorageEntryType {
    /// Plain storage entry (just the value).
    Plain(u32),
    /// A storage map.
    Map {
        /// One or more hashers, should be one hasher per key element.
        hashers: Vec<StorageHasher>,
        /// The type of the key, can be a tuple with elements for each of the hashers.
        key_ty: u32,
        /// The type of the value.
        value_ty: u32,
    },
}

/// Hasher used by storage maps
#[derive(Debug, Clone, Copy)]
pub enum StorageHasher {
    /// 128-bit Blake2 hash.
    Blake2_128,
    /// 256-bit Blake2 hash.
    Blake2_256,
    /// Multiple 128-bit Blake2 hashes concatenated.
    Blake2_128Concat,
    /// 128-bit XX hash.
    Twox128,
    /// 256-bit XX hash.
    Twox256,
    /// Multiple 64-bit XX hashes concatenated.
    Twox64Concat,
    /// Identity hashing (no hashing).
    Identity,
}

#[derive(Debug, Clone, Copy)]
pub enum StorageEntryModifier {
    /// The storage entry returns an `Option<T>`, with `None` if the key is not present.
    Optional,
    /// The storage entry returns `T::Default` if the key is not present.
    Default,
}

#[derive(Debug, Clone)]
pub struct ConstantMetadata {
    /// Name of the pallet constant.
    name: ArcStr,
    /// Type of the pallet constant.
    ty: u32,
    /// Value stored in the constant (SCALE encoded).
    value: Vec<u8>,
    /// Constant documentation.
    docs: Vec<String>,
}

impl ConstantMetadata {
    /// Name of the pallet constant.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Type of the pallet constant.
    pub fn ty(&self) -> u32 {
        self.ty
    }
    /// Value stored in the constant (SCALE encoded).
    pub fn value(&self) -> &[u8] {
        &self.value
    }
    /// Constant documentation.
    pub fn docs(&self) -> &[String] {
        &self.docs
    }
}

#[derive(Debug, Clone)]
pub struct ExtrinsicMetadata {
    /// The type of the extrinsic.
    ty: u32,
    /// Extrinsic version.
    version: u8,
    /// The signed extensions in the order they appear in the extrinsic.
    signed_extensions: Vec<SignedExtensionMetadata>,
}

impl ExtrinsicMetadata {
    /// Type of the extrinsic.
    pub fn ty(&self) -> u32 {
        self.ty
    }

    /// Extrinsic version.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// The extra/additional information associated with the extrinsic.
    pub fn signed_extensions(&self) -> &[SignedExtensionMetadata] {
        &self.signed_extensions
    }
}

#[derive(Debug, Clone)]
pub struct SignedExtensionMetadata {
    /// The unique signed extension identifier, which may be different from the type name.
    identifier: String,
    /// The type of the signed extension, with the data to be included in the extrinsic.
    extra_ty: u32,
    /// The type of the additional signed data, with the data to be included in the signed payload
    additional_ty: u32,
}

impl SignedExtensionMetadata {
    /// The unique signed extension identifier, which may be different from the type name.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
    /// The type of the signed extension, with the data to be included in the extrinsic.
    pub fn extra_ty(&self) -> u32 {
        self.extra_ty
    }
    /// The type of the additional signed data, with the data to be included in the signed payload
    pub fn additional_ty(&self) -> u32 {
        self.additional_ty
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeApiMetadata<'a> {
    inner: &'a RuntimeApiMetadataInner,
    types: &'a PortableRegistry,
}

impl<'a> RuntimeApiMetadata<'a> {
    /// Trait name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }
    /// Trait documentation.
    pub fn docs(&self) -> &[String] {
        &self.inner.docs
    }
    /// An iterator over the trait methods.
    pub fn methods(&self) -> impl Iterator<Item = &'a RuntimeApiMethodMetadata> {
        self.inner.methods.values().iter()
    }
    /// Get a specific trait method given its name.
    pub fn method_by_name(&self, name: &str) -> Option<&'a RuntimeApiMethodMetadata> {
        self.inner.methods.get_by_key(name)
    }
    /// Return a hash for the constant, or None if it was not found.
    pub fn method_hash(&self, method_name: &str) -> Option<[u8; 32]> {
        crate::utils::validation::get_runtime_api_hash(self, method_name)
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeApiMetadataInner {
    /// Trait name.
    name: ArcStr,
    /// Trait methods.
    methods: OrderedMap<ArcStr, RuntimeApiMethodMetadata>,
    /// Trait documentation.
    docs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeApiMethodMetadata {
    /// Method name.
    name: ArcStr,
    /// Method parameters.
    inputs: Vec<RuntimeApiMethodParamMetadata>,
    /// Method output type.
    output_ty: u32,
    /// Method documentation.
    docs: Vec<String>,
}

impl RuntimeApiMethodMetadata {
    /// Method name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Method documentation.
    pub fn docs(&self) -> &[String] {
        &self.docs
    }
    /// Method inputs.
    pub fn inputs(&self) -> impl Iterator<Item = &RuntimeApiMethodParamMetadata> {
        self.inputs.iter()
    }
    /// Method return type.
    pub fn output_ty(&self) -> u32 {
        self.output_ty
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeApiMethodParamMetadata {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub ty: u32,
}
