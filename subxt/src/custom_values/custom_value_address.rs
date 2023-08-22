use std::marker::PhantomData;

use crate::dynamic::DecodedValueThunk;
use crate::metadata::DecodeWithMetadata;

/// This represents the address of a custom value in in the metadata.
/// Anything, that implements the [CustomValueAddress] trait can be used, to fetch
/// custom values from the metadata.
/// The trait is implemented by [str] for dynamic loopup and [StaticAddress] for static queries.
pub trait CustomValueAddress {
    /// The type of the custom value.
    type Target: DecodeWithMetadata;

    /// the name (key) by which the custom value can be accessed in the metadata.
    fn name(&self) -> &str;

    /// An optional hash which, if present, can be checked against node metadata.
    fn validation_hash(&self) -> Option<[u8; 32]> {
        None
    }
}

impl CustomValueAddress for str {
    type Target = DecodedValueThunk;

    fn name(&self) -> &str {
        self
    }
}

/// A static address to a custom value.
pub struct StaticAddress<R> {
    name: &'static str,
    hash: Option<[u8; 32]>,
    phantom: PhantomData<R>,
}

impl<R> StaticAddress<R> {
    #[doc(hidden)]
    /// Creates a new StaticAddress.
    pub fn new_static(name: &'static str, hash: [u8; 32]) -> Self {
        StaticAddress {
            name,
            hash: Some(hash),
            phantom: PhantomData,
        }
    }

    /// Do not validate this custom value prior to accessing it.
    pub fn unvalidated(self) -> Self {
        Self {
            name: self.name,
            hash: None,
            phantom: self.phantom,
        }
    }
}

impl<R: DecodeWithMetadata> CustomValueAddress for StaticAddress<R> {
    type Target = R;

    fn name(&self) -> &str {
        self.name
    }

    fn validation_hash(&self) -> Option<[u8; 32]> {
        self.hash
    }
}
