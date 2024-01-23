// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use crate::error::RpcError;
use futures::Stream;
use std::{future::Future, pin::Pin};

// Re-exporting for simplicity since it's used a bunch in the trait definition.
pub use serde_json::value::RawValue;

/// A trait describing low level JSON-RPC interactions. Implementations of this can be
/// used to instantiate a [`super::RpcClient`], which can be passed to [`crate::OnlineClient`]
/// or used for lower level RPC calls via eg [`crate::backend::legacy::LegacyRpcMethods`].
///
/// This is a low level interface whose methods expect an already-serialized set of params,
/// and return an owned but still-serialized [`RawValue`], deferring deserialization to
/// the caller. This is the case because we want the methods to be object-safe (which prohibits
/// generics), and want to avoid any unnecessary allocations in serializing/deserializing
/// parameters.
///
/// # Panics
///
/// Implementations are free to panic if the `RawValue`'s passed to `request_raw` or
/// `subscribe_raw` are not JSON arrays. Internally, we ensure that this is always the case.
pub trait RpcClientT: Send + Sync + 'static {
    /// Make a raw request for which we expect a single response back from. Implementations
    /// should expect that the params will either be `None`, or be an already-serialized
    /// JSON array of parameters.
    ///
    /// See [`super::RpcParams`] and the [`super::rpc_params!`] macro for an example of how to
    /// construct the parameters.
    ///
    /// Prefer to use the interface provided on [`super::RpcClient`] where possible.
    fn request_raw<'a>(
        &'a self,
        method: &'a str,
        params: Option<Box<RawValue>>,
    ) -> RawRpcFuture<'a, Box<RawValue>>;

    /// Subscribe to some method. Implementations should expect that the params will
    /// either be `None`, or be an already-serialized JSON array of parameters.
    ///
    /// See [`super::RpcParams`] and the [`super::rpc_params!`] macro for an example of how to
    /// construct the parameters.
    ///
    /// Prefer to use the interface provided on [`super::RpcClient`] where possible.
    fn subscribe_raw<'a>(
        &'a self,
        sub: &'a str,
        params: Option<Box<RawValue>>,
        unsub: &'a str,
    ) -> RawRpcFuture<'a, RawRpcSubscription>;
}

/// A boxed future that is returned from the [`RpcClientT`] methods.
pub type RawRpcFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RpcError>> + Send + 'a>>;

/// The RPC subscription returned from [`RpcClientT`]'s `subscription` method.
pub struct RawRpcSubscription {
    /// The subscription stream.
    pub stream: Pin<Box<dyn Stream<Item = Result<Box<RawValue>, RpcError>> + Send + 'static>>,
    /// The ID associated with the subscription.
    pub id: Option<String>,
}

impl<T: RpcClientT> RpcClientT for std::sync::Arc<T> {
    fn request_raw<'a>(
        &'a self,
        method: &'a str,
        params: Option<Box<RawValue>>,
    ) -> RawRpcFuture<'a, Box<RawValue>> {
        (**self).request_raw(method, params)
    }

    fn subscribe_raw<'a>(
        &'a self,
        sub: &'a str,
        params: Option<Box<RawValue>>,
        unsub: &'a str,
    ) -> RawRpcFuture<'a, RawRpcSubscription> {
        (**self).subscribe_raw(sub, params, unsub)
    }
}

impl<T: RpcClientT> RpcClientT for Box<T> {
    fn request_raw<'a>(
        &'a self,
        method: &'a str,
        params: Option<Box<RawValue>>,
    ) -> RawRpcFuture<'a, Box<RawValue>> {
        (**self).request_raw(method, params)
    }

    fn subscribe_raw<'a>(
        &'a self,
        sub: &'a str,
        params: Option<Box<RawValue>>,
        unsub: &'a str,
    ) -> RawRpcFuture<'a, RawRpcSubscription> {
        (**self).subscribe_raw(sub, params, unsub)
    }
}

#[cfg(feature = "reconnecting-rpc-client")]
impl RpcClientT for reconnecting_jsonrpsee_ws_client::Client {
    fn request_raw<'a>(
        &'a self,
        method: &'a str,
        params: Option<Box<RawValue>>,
    ) -> RawRpcFuture<'a, Box<RawValue>> {
        use futures::FutureExt;
        use reconnecting_jsonrpsee_ws_client::RpcParams;

        async {
            self.request_raw(method.to_string(), RpcParams::new(params))
                .await
                .map_err(|e| RpcError::ClientError(Box::new(e)))
        }
        .boxed()
    }

    fn subscribe_raw<'a>(
        &'a self,
        sub: &'a str,
        params: Option<Box<RawValue>>,
        unsub: &'a str,
    ) -> RawRpcFuture<'a, RawRpcSubscription> {
        use futures::{FutureExt, StreamExt, TryStreamExt};
        use reconnecting_jsonrpsee_ws_client::{RpcParams, SubscriptionId};

        async {
            let sub = self
                .subscribe_raw(sub.to_string(), RpcParams::new(params), unsub.to_string())
                .await
                .map_err(|e| RpcError::ClientError(Box::new(e)))?;

            let id = match sub.id() {
                SubscriptionId::Num(n) => n.to_string(),
                SubscriptionId::Str(s) => s.to_string(),
            };
            let stream = sub.map_err(|_| RpcError::DisconnectedWillReconnect).boxed();

            Ok(RawRpcSubscription {
                stream,
                id: Some(id),
            })
        }
        .boxed()
    }
}
