// Copyright 2019-2022 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! A representation of the dispatch error; an error returned when
//! something fails in trying to submit/execute a transaction.

use crate::metadata::{DecodeWithMetadata, Metadata};
use core::fmt::Debug;
use scale_decode::visitor::DecodeAsTypeResult;
use std::borrow::Cow;

/// An error dispatching a transaction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DispatchError {
    /// Some error occurred.
    #[error("Some unknown error occurred.")]
    Other,
    /// Failed to lookup some data.
    #[error("Failed to lookup some data.")]
    CannotLookup,
    /// A bad origin.
    #[error("Bad origin.")]
    BadOrigin,
    /// A custom error in a module.
    #[error("Pallet error: {0}")]
    Module(ModuleError),
    /// At least one consumer is remaining so the account cannot be destroyed.
    #[error("At least one consumer is remaining so the account cannot be destroyed.")]
    ConsumerRemaining,
    /// There are no providers so the account cannot be created.
    #[error("There are no providers so the account cannot be created.")]
    NoProviders,
    /// There are too many consumers so the account cannot be created.
    #[error("There are too many consumers so the account cannot be created.")]
    TooManyConsumers,
    /// An error to do with tokens.
    #[error("Token error: {0}")]
    Token(TokenError),
    /// An arithmetic error.
    #[error("Arithmetic error: {0}")]
    Arithmetic(ArithmeticError),
    /// The number of transactional layers has been reached, or we are not in a transactional layer.
    #[error("Transactional error: {0}")]
    Transactional(TransactionalError),
    /// Resources exhausted, e.g. attempt to read/write data which is too large to manipulate.
    #[error(
        "Resources exhausted, e.g. attempt to read/write data which is too large to manipulate."
    )]
    Exhausted,
    /// The state is corrupt; this is generally not going to fix itself.
    #[error("The state is corrupt; this is generally not going to fix itself.")]
    Corruption,
    /// Some resource (e.g. a preimage) is unavailable right now. This might fix itself later.
    #[error(
        "Some resource (e.g. a preimage) is unavailable right now. This might fix itself later."
    )]
    Unavailable,
}

/// An error relating to tokens when dispatching a transaction.
#[derive(scale_decode::DecodeAsType, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TokenError {
    /// Funds are unavailable.
    #[error("Funds are unavailable.")]
    FundsUnavailable,
    /// Some part of the balance gives the only provider reference to the account and thus cannot be (re)moved.
    #[error("Some part of the balance gives the only provider reference to the account and thus cannot be (re)moved.")]
    OnlyProvider,
    /// Account cannot exist with the funds that would be given.
    #[error("Account cannot exist with the funds that would be given.")]
    BelowMinimum,
    /// Account cannot be created.
    #[error("Account cannot be created.")]
    CannotCreate,
    /// The asset in question is unknown.
    #[error("The asset in question is unknown.")]
    UnknownAsset,
    /// Funds exist but are frozen.
    #[error("Funds exist but are frozen.")]
    Frozen,
    /// Operation is not supported by the asset.
    #[error("Operation is not supported by the asset.")]
    Unsupported,
    /// Account cannot be created for a held balance.
    #[error("Account cannot be created for a held balance.")]
    CannotCreateHold,
    /// Withdrawal would cause unwanted loss of account.
    #[error("Withdrawal would cause unwanted loss of account.")]
    NotExpendable,
}

/// An error relating to arithmetic when dispatching a transaction.
#[derive(scale_decode::DecodeAsType, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArithmeticError {
    /// Underflow.
    #[error("Underflow.")]
    Underflow,
    /// Overflow.
    #[error("Overflow.")]
    Overflow,
    /// Division by zero.
    #[error("Division by zero.")]
    DivisionByZero,
}

/// An error relating to thr transactional layers when dispatching a transaction.
#[derive(scale_decode::DecodeAsType, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransactionalError {
    /// Too many transactional layers have been spawned.
    #[error("Too many transactional layers have been spawned.")]
    LimitReached,
    /// A transactional layer was expected, but does not exist.
    #[error("A transactional layer was expected, but does not exist.")]
    NoLayer,
}

/// Details about a module error that has occurred.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
#[error("{pallet}: {error}\n\n{}", .description.join("\n"))]
pub struct ModuleError {
    /// The name of the pallet that the error came from.
    pub pallet: String,
    /// The name of the error.
    pub error: String,
    /// A description of the error.
    pub description: Vec<String>,
    /// A byte representation of the error.
    pub error_data: RawModuleError,
}

/// The error details about a module error that has occurred.
///
/// **Note**: Structure used to obtain the underlying bytes of a ModuleError.
#[derive(Clone, Debug)]
pub struct RawModuleError {
    /// Index of the pallet that the error came from.
    pub pallet_index: u8,
    /// Raw error bytes.
    pub error: [u8; 4],
}

impl RawModuleError {
    /// Obtain the error index from the underlying byte data.
    pub fn error_index(&self) -> u8 {
        // Error index is utilized as the first byte from the error array.
        self.error[0]
    }
}

impl DispatchError {
    /// Attempt to decode a runtime [`DispatchError`].
    pub fn decode_from<'a>(
        bytes: impl Into<Cow<'a, [u8]>>,
        metadata: &Metadata,
    ) -> Result<Self, super::Error> {
        let bytes = bytes.into();

        let dispatch_error_ty_id = match metadata.dispatch_error_ty() {
            Some(id) => id,
            None => {
                tracing::warn!(
                    "Can't decode error: sp_runtime::DispatchError was not found in Metadata"
                );
                return Err(super::Error::UnknownRuntime(bytes.into_owned()));
            }
        };

        // The aim is to decode our bytes into roughly this shape:
        #[derive(scale_decode::DecodeAsType)]
        enum DecodedDispatchError {
            Other,
            CannotLookup,
            BadOrigin,
            Module(DecodedModuleErrorBytes),
            ConsumerRemaining,
            NoProviders,
            TooManyConsumers,
            Token(TokenError),
            Arithmetic(ArithmeticError),
            Transactional(TransactionalError),
            Exhausted,
            Corruption,
            Unavailable,
        }

        // ModuleError is a bit special; we want to support being decoded from either
        // a legacy format of 2 bytes, or a newer format of 5 bytes. So, just grab the bytes
        // out when decoding to manually work with them.
        struct DecodedModuleErrorBytes(Vec<u8>);
        struct DecodedModuleErrorBytesVisitor;
        impl scale_decode::Visitor for DecodedModuleErrorBytesVisitor {
            type Error = scale_decode::Error;
            type Value<'scale, 'info> = DecodedModuleErrorBytes;
            fn unchecked_decode_as_type<'scale, 'info>(
                self,
                input: &mut &'scale [u8],
                _type_id: scale_decode::visitor::TypeId,
                _types: &'info scale_info::PortableRegistry,
            ) -> DecodeAsTypeResult<Self, Result<Self::Value<'scale, 'info>, Self::Error>>
            {
                DecodeAsTypeResult::Decoded(Ok(DecodedModuleErrorBytes(input.to_vec())))
            }
        }
        impl scale_decode::IntoVisitor for DecodedModuleErrorBytes {
            type Visitor = DecodedModuleErrorBytesVisitor;
            fn into_visitor() -> Self::Visitor {
                DecodedModuleErrorBytesVisitor
            }
        }

        // Decode into our temporary error:
        let decoded_dispatch_err = DecodedDispatchError::decode_with_metadata(
            &mut &*bytes,
            dispatch_error_ty_id,
            metadata,
        )?;

        // Convert into the outward-facing error, mainly by handling the Module variant.
        let dispatch_error = match decoded_dispatch_err {
            // Mostly we don't change anything from our decoded to our outward-facing error:
            DecodedDispatchError::Other => DispatchError::Other,
            DecodedDispatchError::CannotLookup => DispatchError::CannotLookup,
            DecodedDispatchError::BadOrigin => DispatchError::BadOrigin,
            DecodedDispatchError::ConsumerRemaining => DispatchError::ConsumerRemaining,
            DecodedDispatchError::NoProviders => DispatchError::NoProviders,
            DecodedDispatchError::TooManyConsumers => DispatchError::TooManyConsumers,
            DecodedDispatchError::Token(val) => DispatchError::Token(val),
            DecodedDispatchError::Arithmetic(val) => DispatchError::Arithmetic(val),
            DecodedDispatchError::Transactional(val) => DispatchError::Transactional(val),
            DecodedDispatchError::Exhausted => DispatchError::Exhausted,
            DecodedDispatchError::Corruption => DispatchError::Corruption,
            DecodedDispatchError::Unavailable => DispatchError::Unavailable,
            // But we apply custom logic to transform the module error into the outward facing version:
            DecodedDispatchError::Module(bytes) => {
                let bytes = bytes.0;

                // The old version is 2 bytes; a pallet and error index.
                // The new version is 5 bytes; a pallet and error index and then 3 extra bytes.
                let err = if bytes.len() == 2 {
                    RawModuleError {
                        pallet_index: bytes[0],
                        error: [bytes[1], 0, 0, 0],
                    }
                } else if bytes.len() == 5 {
                    RawModuleError {
                        pallet_index: bytes[0],
                        error: [bytes[1], bytes[2], bytes[3], bytes[4]],
                    }
                } else {
                    tracing::warn!("Can't decode error: sp_runtime::DispatchError::Module bytes do not match known shapes");
                    return Err(super::Error::UnknownRuntime(bytes));
                };

                // Embelish the error with extra info helpful for matching it up etc:
                let error_details = match metadata.error(err.pallet_index, err.error[0]) {
                    Ok(details) => details,
                    Err(_) => {
                        tracing::warn!("Can't embelish error: sp_runtime::DispatchError::Module details do not match known information");
                        return Err(super::Error::UnknownRuntime(bytes));
                    }
                };

                // And return our outward-facing version:
                DispatchError::Module(ModuleError {
                    pallet: error_details.pallet().to_string(),
                    error: error_details.error().to_string(),
                    description: error_details.docs().to_vec(),
                    error_data: err,
                })
            }
        };

        Ok(dispatch_error)
    }
}
