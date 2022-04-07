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

use codec::Encode;
use frame_metadata::{
    ExtrinsicMetadata,
    RuntimeMetadataLastVersion,
    StorageEntryType, StorageEntryMetadata,
};
use scale_info::{
    form::PortableForm,
    Field,
    PortableRegistry,
    TypeDef,
    Variant,
};
use std::collections::{
    HashMap,
    HashSet,
};

/// Internal byte representation for various metadata types utilized for
/// generating deterministic hashes between different rust versions.
#[repr(u8)]
enum MetadataHashableIDs {
    Field,
    Variant,
    TypeDef,
    Type,
    Pallet,
    Extrinsic,
}

/// Hashing function utilized internally.
fn hash(bytes: &[u8]) -> [u8; 32] {
    sp_core::hashing::sha2_256(bytes)
}

/// Obtain the hash representation of a `scale_info::Field`.
fn get_field_hash(
    registry: &PortableRegistry,
    field: &Field<PortableForm>,
    visited_ids: &mut HashSet<u32>,
) -> [u8; 32] {
    let mut bytes = vec![MetadataHashableIDs::Field as u8];

    field.name().encode_to(&mut bytes);
    field.type_name().encode_to(&mut bytes);
    bytes.extend(get_type_hash(registry, field.ty().id(), visited_ids));

    hash(&bytes)
}

/// Obtain the hash representation of a `scale_info::Variant`.
fn get_variant_hash(
    registry: &PortableRegistry,
    var: &Variant<PortableForm>,
    visited_ids: &mut HashSet<u32>,
) -> [u8; 32] {
    let mut bytes = vec![MetadataHashableIDs::Variant as u8];

    var.name().encode_to(&mut bytes);
    for field in var.fields() {
        bytes.extend(get_field_hash(registry, field, visited_ids));
    }

    hash(&bytes)
}

/// Obtain the hash representation of a `scale_info::TypeDef`.
fn get_type_def_hash(
    registry: &PortableRegistry,
    ty_def: &TypeDef<PortableForm>,
    visited_ids: &mut HashSet<u32>,
) -> [u8; 32] {
    let mut bytes = vec![MetadataHashableIDs::TypeDef as u8];

    let data = match ty_def {
        TypeDef::Composite(composite) => {
            let mut bytes = Vec::new();
            for field in composite.fields() {
                bytes.extend(get_field_hash(registry, field, visited_ids));
            }
            bytes
        }
        TypeDef::Variant(variant) => {
            let mut bytes = Vec::new();
            for var in variant.variants().iter() {
                bytes.extend(get_variant_hash(registry, var, visited_ids));
            }
            bytes
        }
        TypeDef::Sequence(sequence) => {
            let mut bytes = Vec::new();
            bytes.extend(get_type_hash(
                registry,
                sequence.type_param().id(),
                visited_ids,
            ));
            bytes
        }
        TypeDef::Array(array) => {
            let mut bytes = Vec::new();
            array.len().encode_to(&mut bytes);
            bytes.extend(get_type_hash(
                registry,
                array.type_param().id(),
                visited_ids,
            ));
            bytes
        }
        TypeDef::Tuple(tuple) => {
            let mut bytes = Vec::new();
            for field in tuple.fields() {
                bytes.extend(get_type_hash(registry, field.id(), visited_ids));
            }
            bytes
        }
        TypeDef::Primitive(primitive) => {
            let mut bytes = Vec::new();
            primitive.encode_to(&mut bytes);
            bytes
        }
        TypeDef::Compact(compact) => {
            let mut bytes = Vec::new();
            bytes.extend(get_type_hash(
                registry,
                compact.type_param().id(),
                visited_ids,
            ));
            bytes
        }
        TypeDef::BitSequence(bitseq) => {
            let mut bytes = Vec::new();
            bytes.extend(get_type_hash(
                registry,
                bitseq.bit_order_type().id(),
                visited_ids,
            ));
            bytes.extend(get_type_hash(
                registry,
                bitseq.bit_store_type().id(),
                visited_ids,
            ));
            bytes
        }
    };
    bytes.extend(data);
    hash(&bytes)
}

/// Obtain the hash representation of a `scale_info::Type` identified by id.
fn get_type_hash(
    registry: &PortableRegistry,
    id: u32,
    visited_ids: &mut HashSet<u32>,
) -> [u8; 32] {
    let ty = registry.resolve(id).unwrap();

    let mut bytes = vec![MetadataHashableIDs::Type as u8];
    ty.path().segments().encode_to(&mut bytes);
    // Guard against recursive types
    if !visited_ids.insert(id) {
        return hash(&bytes)
    }

    let ty_def = ty.type_def();
    bytes.extend(get_type_def_hash(registry, ty_def, visited_ids));

    hash(&bytes)
}

/// Obtain the hash representation of a `frame_metadata::ExtrinsicMetadata`.
fn get_extrinsic_hash(
    registry: &PortableRegistry,
    extrinsic: &ExtrinsicMetadata<PortableForm>,
) -> [u8; 32] {
    let mut visited_ids = HashSet::<u32>::new();
    let mut bytes = vec![MetadataHashableIDs::Extrinsic as u8];

    bytes.extend(get_type_hash(registry, extrinsic.ty.id(), &mut visited_ids));
    bytes.push(extrinsic.version);
    for signed_extension in extrinsic.signed_extensions.iter() {
        signed_extension.identifier.encode_to(&mut bytes);
        bytes.extend(get_type_hash(
            registry,
            signed_extension.ty.id(),
            &mut visited_ids,
        ));
        bytes.extend(get_type_hash(
            registry,
            signed_extension.additional_signed.id(),
            &mut visited_ids,
        ));
    }

    hash(&bytes)
}

/// Get the hash corresponding to a single storage entry.
fn get_storage_entry_hash(
    registry: &PortableRegistry,
    entry: &StorageEntryMetadata<PortableForm>,
    visited_ids: &mut HashSet<u32>
) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend(entry.name.as_bytes());
    entry.modifier.encode_to(&mut bytes);
    match &entry.ty {
        StorageEntryType::Plain(ty) => {
            bytes.extend(get_type_hash(registry, ty.id(), visited_ids));
        }
        StorageEntryType::Map {
            hashers,
            key,
            value,
        } => {
            hashers.encode_to(&mut bytes);
            bytes.extend(get_type_hash(registry, key.id(), visited_ids));
            bytes.extend(get_type_hash(registry, value.id(), visited_ids));
        }
    }
    bytes.extend(&entry.default);
    hash(&bytes)
}

/// Obtain the hash for a specific storage item, or an error if it's not found.
pub fn get_storage_hash(
    metadata: &RuntimeMetadataLastVersion,
    pallet_name: &str,
    storage_name: &str
) -> Result<[u8; 32], NotFound> {
    let pallet = metadata
        .pallets
        .iter()
        .find(|p| p.name == pallet_name)
        .ok_or(NotFound::Pallet)?;

    let storage = pallet
        .storage
        .as_ref()
        .ok_or(NotFound::Item)?;

    let entry = storage
        .entries
        .iter()
        .find(|s| s.name == storage_name)
        .ok_or(NotFound::Item)?;

    let hash = get_storage_entry_hash(&metadata.types, entry, &mut HashSet::new());
    Ok(hash)
}

/// Obtain the hash for a specific constant, or an error if it's not found.
pub fn get_constant_hash(
    metadata: &RuntimeMetadataLastVersion,
    pallet_name: &str,
    constant_name: &str
) -> Result<[u8; 32], NotFound> {
    let pallet = metadata
        .pallets
        .iter()
        .find(|p| p.name == pallet_name)
        .ok_or(NotFound::Pallet)?;

    let constant = pallet
        .constants
        .iter()
        .find(|c| c.name == constant_name)
        .ok_or(NotFound::Item)?;

    let hash = get_type_hash(&metadata.types, constant.ty.id(), &mut HashSet::new());
    Ok(hash)
}

/// Obtain the hash for a specific call, or an error if it's not found.
pub fn get_call_hash(
    metadata: &RuntimeMetadataLastVersion,
    pallet_name: &str,
    call_name: &str
) -> Result<[u8; 32], NotFound> {
    let pallet = metadata
        .pallets
        .iter()
        .find(|p| p.name == pallet_name)
        .ok_or(NotFound::Pallet)?;

    let call_id = pallet
        .calls
        .as_ref()
        .ok_or(NotFound::Item)?
        .ty
        .id();

    let call_ty = metadata.types
        .resolve(call_id)
        .ok_or(NotFound::Item)?;

    let call_variants = match call_ty.type_def() {
        TypeDef::Variant(variant) => variant.variants(),
        _ => return Err(NotFound::Item)
    };

    let variant = call_variants
        .iter()
        .find(|v| v.name() == call_name)
        .ok_or(NotFound::Item)?;

    // hash the specific variant representing the call we are interested in.
    let hash = get_variant_hash(&metadata.types, variant, &mut HashSet::new());
    Ok(hash)
}

/// Obtain the hash representation of a `frame_metadata::PalletMetadata`.
pub fn get_pallet_hash(
    registry: &PortableRegistry,
    pallet: &frame_metadata::PalletMetadata<PortableForm>,
) -> [u8; 32] {
    let mut bytes = vec![MetadataHashableIDs::Pallet as u8];
    let mut visited_ids = HashSet::<u32>::new();

    if let Some(ref calls) = pallet.calls {
        bytes.extend(get_type_hash(registry, calls.ty.id(), &mut visited_ids));
    }
    if let Some(ref event) = pallet.event {
        bytes.extend(get_type_hash(registry, event.ty.id(), &mut visited_ids));
    }
    for constant in pallet.constants.iter() {
        bytes.extend(constant.name.as_bytes());
        bytes.extend(&constant.value);
        bytes.extend(get_type_hash(registry, constant.ty.id(), &mut visited_ids));
    }
    if let Some(ref error) = pallet.error {
        bytes.extend(get_type_hash(registry, error.ty.id(), &mut visited_ids));
    }
    if let Some(ref storage) = pallet.storage {
        bytes.extend(storage.prefix.as_bytes());
        for entry in storage.entries.iter() {
            bytes.extend(get_storage_entry_hash(registry, entry, &mut visited_ids));
        }
    }

    hash(&bytes)
}

/// Obtain the hash representation of a `frame_metadata::RuntimeMetadataLastVersion`.
pub fn get_metadata_hash(metadata: &RuntimeMetadataLastVersion) -> MetadataHashDetails {
    // Collect all pairs of (pallet name, pallet hash).
    let mut pallets: Vec<(String, [u8; 32])> = metadata
        .pallets
        .iter()
        .map(|pallet| {
            let name = pallet.name.clone();
            let hash = get_pallet_hash(&metadata.types, pallet);
            (name, hash)
        })
        .collect();
    // Sort by pallet name to create a deterministic representation of the underlying metadata.
    pallets.sort_by_key(|key| key.1);

    // Note: pallet name is excluded from hashing.
    // Each pallet has a hash of 32 bytes, and the vector is extended with
    // extrinsic hash and metadata ty hash (2 * 32).
    let mut bytes = Vec::with_capacity(pallets.len() * 32 + 64);
    for (_, hash) in pallets.iter() {
        bytes.extend(hash)
    }

    bytes.extend(get_extrinsic_hash(&metadata.types, &metadata.extrinsic));

    let mut visited_ids = HashSet::<u32>::new();
    bytes.extend(get_type_hash(
        &metadata.types,
        metadata.ty.id(),
        &mut visited_ids,
    ));

    let hash = hash(&bytes);

    MetadataHashDetails {
        metadata_hash: hash,
        pallet_hashes: pallets.into_iter().collect(),
    }
}

/// Obtain the hash representation of a `frame_metadata::RuntimeMetadataLastVersion`
/// hashing only the provided pallets.
///
/// **Note:** This is similar to `get_metadata_hash`, but performs hashing only of the provided
/// pallets if they exist. There are cases where the runtime metadata contains a subset of
/// the pallets from the static metadata. In those cases, the static API can communicate
/// properly with the subset of pallets from the runtime node.
pub fn get_metadata_per_pallet_hash<T: AsRef<str>>(
    metadata: &RuntimeMetadataLastVersion,
    pallets: &[T],
) -> MetadataHashDetails {
    // Collect all pairs of (pallet name, pallet hash).
    let mut pallets_hashed: Vec<(String, [u8; 32])> = metadata
        .pallets
        .iter()
        .filter_map(|pallet| {
            // Make sure to filter just the pallets we are interested in.
            let in_pallet = pallets
                .iter()
                .any(|pallet_ref| pallet_ref.as_ref() == pallet.name);
            if in_pallet {
                let name = pallet.name.clone();
                let hash = get_pallet_hash(&metadata.types, pallet);
                Some((name, hash))
            } else {
                None
            }
        })
        .collect();
    // Sort by pallet name to create a deterministic representation of the underlying metadata.
    pallets_hashed.sort_by_key(|key| key.1);
    // Note: pallet name is excluded from hashing.
    // Each pallet has a hash of 32 bytes, and the vector is extended with
    // extrinsic hash and metadata ty hash (2 * 32).
    let mut bytes = Vec::with_capacity(pallets_hashed.len() * 32);
    for (_, hash) in pallets_hashed.iter() {
        bytes.extend(hash)
    }

    let hash = hash(&bytes);

    MetadataHashDetails {
        metadata_hash: hash,
        pallet_hashes: pallets_hashed.into_iter().collect(),
    }
}

/// Metadata hash details obtained from hashing `frame_metadata::RuntimeMetadataLastVersion`.
///
/// **Note:** This structure provides a caching mechanism when the customer
/// is obtaining the full metadata hash first, then the pallet hash.
#[derive(Clone, Debug)]
pub struct MetadataHashDetails {
    /// Full metadata hash.
    pub metadata_hash: [u8; 32],
    /// Pallet hashes resulted from metadata hashing.
    pub pallet_hashes: HashMap<String, [u8; 32]>,
}

impl MetadataHashDetails {
    /// Creates an empty `MetadataHashDetails`.
    pub fn new() -> MetadataHashDetails {
        Self {
            metadata_hash: [0; 32],
            pallet_hashes: HashMap::new(),
        }
    }
}

impl Default for MetadataHashDetails {
    fn default() -> Self {
        Self::new()
    }
}

/// An error returned if we attempt to get the hash for a specific call, constant
/// or storage item that doesn't exist.
#[derive(Clone, Debug)]
pub enum NotFound {
    Pallet,
    Item,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitvec::{
        order::Lsb0,
        vec::BitVec,
    };
    use frame_metadata::{
        ExtrinsicMetadata,
        PalletCallMetadata,
        PalletConstantMetadata,
        PalletErrorMetadata,
        PalletEventMetadata,
        PalletMetadata,
        PalletStorageMetadata,
        RuntimeMetadataLastVersion,
        StorageEntryMetadata,
        StorageEntryModifier,
    };
    use scale_info::meta_type;

    // Define recursive types.
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    struct A {
        pub b: Box<B>,
    }
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    struct B {
        pub a: Box<A>,
    }

    // Define TypeDef supported types.
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    // TypeDef::Composite with TypeDef::Array with Typedef::Primitive.
    struct AccountId32([u8; 32]);
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    // TypeDef::Variant.
    enum DigestItem {
        PreRuntime(
            // TypeDef::Array with primitive.
            [::core::primitive::u8; 4usize],
            // TypeDef::Sequence.
            ::std::vec::Vec<::core::primitive::u8>,
        ),
        Other(::std::vec::Vec<::core::primitive::u8>),
        // Nested TypeDef::Tuple.
        RuntimeEnvironmentUpdated(((i8, i16), (u32, u64))),
        // TypeDef::Compact.
        Index(#[codec(compact)] ::core::primitive::u8),
        // TypeDef::BitSequence.
        BitSeq(BitVec<u8, Lsb0>),
    }
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    // Ensure recursive types and TypeDef variants are captured.
    struct MetadataTestType {
        recursive: A,
        composite: AccountId32,
        type_def: DigestItem,
    }
    #[allow(dead_code)]
    #[derive(scale_info::TypeInfo)]
    // Simulate a PalletCallMetadata.
    enum Call {
        #[codec(index = 0)]
        FillBlock { ratio: AccountId32 },
        #[codec(index = 1)]
        Remark { remark: DigestItem },
    }

    fn build_default_extrinsic() -> ExtrinsicMetadata {
        ExtrinsicMetadata {
            ty: meta_type::<()>(),
            version: 0,
            signed_extensions: vec![],
        }
    }

    fn default_pallet() -> PalletMetadata {
        PalletMetadata {
            name: "Test",
            storage: None,
            calls: None,
            event: None,
            constants: vec![],
            error: None,
            index: 0,
        }
    }

    fn build_default_pallets() -> Vec<PalletMetadata> {
        vec![
            PalletMetadata {
                name: "First",
                calls: Some(PalletCallMetadata {
                    ty: meta_type::<MetadataTestType>(),
                }),
                ..default_pallet()
            },
            PalletMetadata {
                name: "Second",
                index: 1,
                calls: Some(PalletCallMetadata {
                    ty: meta_type::<(DigestItem, AccountId32, A)>(),
                }),
                ..default_pallet()
            },
        ]
    }

    fn pallets_to_metadata(pallets: Vec<PalletMetadata>) -> RuntimeMetadataLastVersion {
        RuntimeMetadataLastVersion::new(
            pallets,
            build_default_extrinsic(),
            meta_type::<()>(),
        )
    }

    #[test]
    fn different_pallet_index() {
        let pallets = build_default_pallets();
        let mut pallets_swap = pallets.clone();

        let metadata = pallets_to_metadata(pallets);

        // Change the order in which pallets are registered.
        pallets_swap.swap(0, 1);
        pallets_swap[0].index = 0;
        pallets_swap[1].index = 1;
        let metadata_swap = pallets_to_metadata(pallets_swap);

        let hash = get_metadata_hash(&metadata);
        let hash_swap = get_metadata_hash(&metadata_swap);

        // Changing pallet order must still result in a deterministic unique hash.
        assert_eq!(hash.metadata_hash, hash_swap.metadata_hash);
        assert_eq!(hash.pallet_hashes, hash_swap.pallet_hashes);

        // Verify a fresh compute with cached pallet hash.
        let pallet_hash_first = get_pallet_hash(&metadata.types, &metadata.pallets[0]);
        assert_eq!(hash.pallet_hashes.get("First").unwrap(), &pallet_hash_first);

        let pallet_hash_second = get_pallet_hash(&metadata.types, &metadata.pallets[1]);
        assert_eq!(
            hash.pallet_hashes.get("Second").unwrap(),
            &pallet_hash_second
        );
    }

    #[test]
    fn recursive_type() {
        let mut pallet = default_pallet();
        pallet.calls = Some(PalletCallMetadata {
            ty: meta_type::<A>(),
        });
        let metadata = pallets_to_metadata(vec![pallet]);

        // Check hashing algorithm finishes on a recursive type.
        let hash = get_metadata_hash(&metadata);

        // Verify a fresh compute with cached pallet hash.
        let pallet_hash = get_pallet_hash(&metadata.types, &metadata.pallets[0]);
        assert_eq!(hash.pallet_hashes.get("Test").unwrap(), &pallet_hash);
    }

    #[test]
    /// Ensure correctness of hashing when parsing the `metadata.types`.
    ///
    /// Having a recursive structure `A: { B }` and `B: { A }` registered in different order
    /// `types: { { id: 0, A }, { id: 1, B } }` and `types: { { id: 0, B }, { id: 1, A } }`
    /// must produce the same deterministic hashing value.
    fn recursive_types_different_order() {
        let mut pallets = build_default_pallets();
        pallets[0].calls = Some(PalletCallMetadata {
            ty: meta_type::<A>(),
        });
        pallets[1].calls = Some(PalletCallMetadata {
            ty: meta_type::<B>(),
        });
        pallets[1].index = 1;
        let mut pallets_swap = pallets.clone();
        let metadata = pallets_to_metadata(pallets);

        pallets_swap.swap(0, 1);
        pallets_swap[0].index = 0;
        pallets_swap[1].index = 1;
        let metadata_swap = pallets_to_metadata(pallets_swap);

        let hash = get_metadata_hash(&metadata);
        let hash_swap = get_metadata_hash(&metadata_swap);

        // Changing pallet order must still result in a deterministic unique hash.
        assert_eq!(hash.metadata_hash, hash_swap.metadata_hash);
        assert_eq!(hash.pallet_hashes, hash_swap.pallet_hashes);

        // Verify a fresh compute with cached pallet hash.
        let pallet_hash_first = get_pallet_hash(&metadata.types, &metadata.pallets[0]);
        assert_eq!(hash.pallet_hashes.get("First").unwrap(), &pallet_hash_first);

        let pallet_hash_second = get_pallet_hash(&metadata.types, &metadata.pallets[1]);
        assert_eq!(
            hash.pallet_hashes.get("Second").unwrap(),
            &pallet_hash_second
        );
    }

    #[test]
    fn pallet_hash_correctness() {
        let compare_pallets_hash = |lhs: &PalletMetadata, rhs: &PalletMetadata| {
            let metadata = pallets_to_metadata(vec![lhs.clone()]);
            let hash = get_metadata_hash(&metadata);

            let metadata = pallets_to_metadata(vec![rhs.clone()]);
            let new_hash = get_metadata_hash(&metadata);

            assert_ne!(hash.metadata_hash, new_hash.metadata_hash);
            assert_ne!(
                hash.pallet_hashes.get("Test").unwrap(),
                new_hash.pallet_hashes.get("Test").unwrap()
            );
        };

        // Build metadata progressively from an empty pallet to a fully populated pallet.
        let mut pallet = default_pallet();
        let pallet_lhs = pallet.clone();
        pallet.storage = Some(PalletStorageMetadata {
            prefix: "Storage",
            entries: vec![StorageEntryMetadata {
                name: "BlockWeight",
                modifier: StorageEntryModifier::Default,
                ty: StorageEntryType::Plain(meta_type::<u8>()),
                default: vec![],
                docs: vec![],
            }],
        });
        compare_pallets_hash(&pallet_lhs, &pallet);

        let pallet_lhs = pallet.clone();
        // Calls are similar to:
        //
        // ```
        // pub enum Call {
        //     call_name_01 { arg01: type },
        //     call_name_02 { arg01: type, arg02: type }
        // }
        // ```
        pallet.calls = Some(PalletCallMetadata {
            ty: meta_type::<Call>(),
        });
        compare_pallets_hash(&pallet_lhs, &pallet);

        let pallet_lhs = pallet.clone();
        // Events are similar to Calls.
        pallet.event = Some(PalletEventMetadata {
            ty: meta_type::<Call>(),
        });
        compare_pallets_hash(&pallet_lhs, &pallet);

        let pallet_lhs = pallet.clone();
        pallet.constants = vec![PalletConstantMetadata {
            name: "BlockHashCount",
            ty: meta_type::<u64>(),
            value: vec![96u8, 0, 0, 0],
            docs: vec![],
        }];
        compare_pallets_hash(&pallet_lhs, &pallet);

        let pallet_lhs = pallet.clone();
        pallet.error = Some(PalletErrorMetadata {
            ty: meta_type::<MetadataTestType>(),
        });
        compare_pallets_hash(&pallet_lhs, &pallet);
    }

    #[test]
    fn metadata_per_pallet_hash_correctness() {
        let pallets = build_default_pallets();

        // Build metadata with just the first pallet.
        let metadata = pallets_to_metadata(vec![pallets[0].clone()]);
        // Build metadata with both pallets.
        let metadata_second = pallets_to_metadata(pallets);

        // Check hashing with non-existent "Second" pallet.
        let hash = get_metadata_per_pallet_hash(&metadata, &["First", "Second"]);
        assert!(hash.pallet_hashes.get("First").is_some());
        assert!(hash.pallet_hashes.get("Second").is_none());

        // Check hashing with existing "First" pallet, ensure hashing is consistent.
        let hash_rhs = get_metadata_per_pallet_hash(&metadata, &["First"]);
        assert!(hash_rhs.pallet_hashes.get("First").is_some());
        assert!(hash_rhs.pallet_hashes.get("Second").is_none());
        assert_eq!(hash.metadata_hash, hash_rhs.metadata_hash);
        assert_eq!(
            hash.pallet_hashes.get("First").unwrap(),
            hash_rhs.pallet_hashes.get("First").unwrap()
        );

        // Check hashing with one pallet out of two.
        let hash_second = get_metadata_per_pallet_hash(&metadata_second, &["First"]);
        assert!(hash_second.pallet_hashes.get("First").is_some());
        assert!(hash_second.pallet_hashes.get("Second").is_none());
        assert_eq!(hash_second.metadata_hash, hash.metadata_hash);
        assert_eq!(
            hash_second.pallet_hashes.get("First").unwrap(),
            hash.pallet_hashes.get("First").unwrap()
        );

        // Check hashing with all pallets.
        let hash_second =
            get_metadata_per_pallet_hash(&metadata_second, &["First", "Second"]);
        assert!(hash_second.pallet_hashes.get("First").is_some());
        assert!(hash_second.pallet_hashes.get("Second").is_some());
        assert_ne!(hash_second.metadata_hash, hash.metadata_hash);
        assert_eq!(
            hash_second.pallet_hashes.get("First").unwrap(),
            hash.pallet_hashes.get("First").unwrap()
        );
    }
}
