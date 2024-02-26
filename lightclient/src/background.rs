// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use futures::stream::StreamExt;
use futures_util::future::{self, Either};
use serde::Deserialize;
use serde_json::value::RawValue;
use smoldot_light::platform::PlatformRef;
use std::{collections::HashMap, str::FromStr};
use tokio::sync::{mpsc, oneshot};

use crate::client::AddedChain;

use super::LightClientRpcError;
use smoldot_light::ChainId;

const LOG_TARGET: &str = "subxt-light-client-background";

/// The response of an RPC method.
pub type MethodResponse = Result<Box<RawValue>, LightClientRpcError>;

/// Message protocol between the front-end client that submits the RPC requests
/// and the backend handler that produces responses from the chain.
///
/// The light client uses a single object [`smoldot_light::JsonRpcResponses`] to
/// handle all requests and subscriptions from a chain. A background task is spawned
/// to multiplex the rpc responses and to provide them back to their rightful submitters.
#[derive(Debug)]
pub enum FromSubxt {
    /// The RPC method request.
    Request {
        /// The method of the request.
        method: String,
        /// The parameters of the request.
        params: String,
        /// Channel used to send back the result.
        sender: oneshot::Sender<MethodResponse>,
        /// The ID of the chain used to identify the chain.
        chain_id: ChainId,
    },
    /// The RPC subscription (pub/sub) request.
    Subscription {
        /// The method of the request.
        method: String,
        /// The method to unsubscribe.
        unsubscribe_method: String,
        /// The parameters of the request.
        params: String,
        /// Channel used to send back the subscription ID if successful.
        sub_id: oneshot::Sender<MethodResponse>,
        /// Channel used to send back the notifications.
        sender: mpsc::UnboundedSender<Box<RawValue>>,
        /// The ID of the chain used to identify the chain.
        chain_id: ChainId,
    },
}

/// Background task data.
#[allow(clippy::type_complexity)]
pub struct BackgroundTask<TPlatform: PlatformRef, TChain> {
    /// Smoldot light client implementation that leverages the exposed platform.
    client: smoldot_light::Client<TPlatform, TChain>,
    /// Per-chain data.
    chain_data: HashMap<smoldot_light::ChainId, ChainData>,
}

/// The data that we store for each chain.
#[derive(Default)]
struct ChainData {
    /// Generates an unique monotonically increasing ID for each chain.
    last_request_id: usize,
    /// Map the request ID of a RPC method to the frontend `Sender`.
    requests: HashMap<usize, oneshot::Sender<MethodResponse>>,
    /// Subscription calls first need to make a plain RPC method
    /// request to obtain the subscription ID.
    ///
    /// The RPC method request is made in the background and the response should
    /// not be sent back to the user.
    /// Map the request ID of a RPC method to the frontend `Sender`.
    id_to_subscription: HashMap<usize, PendingSubscription>,
    /// Map the subscription ID to the frontend `Sender`.
    ///
    /// The subscription ID is entirely generated by the node (smoldot). Therefore, it is
    /// possible for two distinct subscriptions of different chains to have the same subscription ID.
    subscriptions: HashMap<usize, ActiveSubscription>,
}

impl ChainData {
    /// Fetch and increment the request ID.
    fn next_id(&mut self) -> usize {
        self.last_request_id = self.last_request_id.wrapping_add(1);
        self.last_request_id
    }
}

/// The state needed to resolve the subscription ID and send
/// back the response to frontend.
struct PendingSubscription {
    /// Send the method response ID back to the user.
    ///
    /// It contains the subscription ID if successful, or an JSON RPC error object.
    sub_id_sender: oneshot::Sender<MethodResponse>,
    /// The subscription state that is added to the `subscriptions` map only
    /// if the subscription ID is successfully sent back to the user.
    subscription_state: ActiveSubscription,
}

impl PendingSubscription {
    /// Transforms the pending subscription into an active subscription.
    fn into_parts(self) -> (oneshot::Sender<MethodResponse>, ActiveSubscription) {
        (self.sub_id_sender, self.subscription_state)
    }
}

/// The state of the subscription.
struct ActiveSubscription {
    /// Channel to send the subscription notifications back to frontend.
    sender: mpsc::UnboundedSender<Box<RawValue>>,
    /// The unsubscribe method to call when the user drops the receiver
    /// part of the channel.
    unsubscribe_method: String,
}

impl<TPlatform: PlatformRef, TChain> BackgroundTask<TPlatform, TChain> {
    /// Constructs a new [`BackgroundTask`].
    pub fn new(
        client: smoldot_light::Client<TPlatform, TChain>,
    ) -> BackgroundTask<TPlatform, TChain> {
        BackgroundTask {
            client,
            chain_data: Default::default(),
        }
    }

    fn for_chain_id(
        &mut self,
        chain_id: smoldot_light::ChainId,
    ) -> (
        &mut ChainData,
        &mut smoldot_light::Client<TPlatform, TChain>,
    ) {
        let chain_data = self.chain_data.entry(chain_id).or_default();
        let client = &mut self.client;
        (chain_data, client)
    }

    /// Handle the registration messages received from the user.
    async fn handle_requests(&mut self, message: FromSubxt) {
        match message {
            FromSubxt::Request {
                method,
                params,
                sender,
                chain_id,
            } => {
                let (data, client) = self.for_chain_id(chain_id);
                let id = data.next_id();

                let request = format!(
                    r#"{{"jsonrpc":"2.0","id":"{}", "method":"{}","params":{}}}"#,
                    id, method, params
                );

                data.requests.insert(id, sender);
                tracing::trace!(target: LOG_TARGET, "Tracking request id={id} chain={chain_id:?}");

                let result = client.json_rpc_request(request, chain_id);
                if let Err(err) = result {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Cannot send RPC request to lightclient {:?}",
                        err.to_string()
                    );

                    let sender = data
                        .requests
                        .remove(&id)
                        .expect("Channel is inserted above; qed");

                    // Send the error back to frontend.
                    if sender
                        .send(Err(LightClientRpcError::Request(err.to_string())))
                        .is_err()
                    {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send RPC request error to id={id}",
                        );
                    }
                } else {
                    tracing::trace!(target: LOG_TARGET, "Submitted to smoldot request with id={id}");
                }
            }
            FromSubxt::Subscription {
                method,
                unsubscribe_method,
                params,
                sub_id,
                sender,
                chain_id,
            } => {
                let (data, client) = self.for_chain_id(chain_id);
                let id = data.next_id();

                // For subscriptions we need to make a plain RPC request to the subscription method.
                // The server will return as a result the subscription ID.
                let request = format!(
                    r#"{{"jsonrpc":"2.0","id":"{}", "method":"{}","params":{}}}"#,
                    id, method, params
                );

                tracing::trace!(target: LOG_TARGET, "Tracking subscription request id={id} chain={chain_id:?}");
                let subscription_id_state = PendingSubscription {
                    sub_id_sender: sub_id,
                    subscription_state: ActiveSubscription {
                        sender,
                        unsubscribe_method,
                    },
                };
                data.id_to_subscription.insert(id, subscription_id_state);

                let result = client.json_rpc_request(request, chain_id);
                if let Err(err) = result {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Cannot send RPC request to lightclient {:?}",
                        err.to_string()
                    );
                    let subscription_id_state = data
                        .id_to_subscription
                        .remove(&id)
                        .expect("Channels are inserted above; qed");

                    // Send the error back to frontend.
                    if subscription_id_state
                        .sub_id_sender
                        .send(Err(LightClientRpcError::Request(err.to_string())))
                        .is_err()
                    {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send RPC request error to id={id}",
                        );
                    }
                } else {
                    tracing::trace!(target: LOG_TARGET, "Submitted to smoldot subscription request with id={id}");
                }
            }
        };
    }

    /// Parse the response received from the light client and sent it to the appropriate user.
    fn handle_rpc_response(&mut self, chain_id: smoldot_light::ChainId, response: String) {
        tracing::trace!(target: LOG_TARGET, "Received from smoldot response={response} chain={chain_id:?}");
        let (data, _client) = self.for_chain_id(chain_id);

        match RpcResponse::from_str(&response) {
            Ok(RpcResponse::Error { id, error }) => {
                let Ok(id) = id.parse::<usize>() else {
                    tracing::warn!(target: LOG_TARGET, "Cannot send error. Id={id} chain={chain_id:?} is not a valid number");
                    return;
                };

                if let Some(sender) = data.requests.remove(&id) {
                    if sender
                        .send(Err(LightClientRpcError::Request(error.to_string())))
                        .is_err()
                    {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send method response to id={id} chain={chain_id:?}",
                        );
                    }
                } else if let Some(subscription_id_state) = data.id_to_subscription.remove(&id) {
                    if subscription_id_state
                        .sub_id_sender
                        .send(Err(LightClientRpcError::Request(error.to_string())))
                        .is_err()
                    {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send method response to id {id} chain={chain_id:?}",
                        );
                    }
                }
            }
            Ok(RpcResponse::Method { id, result }) => {
                let Ok(id) = id.parse::<usize>() else {
                    tracing::warn!(target: LOG_TARGET, "Cannot send response. Id={id} chain={chain_id:?} is not a valid number");
                    return;
                };

                // Send the response back.
                if let Some(sender) = data.requests.remove(&id) {
                    if sender.send(Ok(result)).is_err() {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send method response to id={id} chain={chain_id:?}",
                        );
                    }
                } else if let Some(pending_subscription) = data.id_to_subscription.remove(&id) {
                    let Ok(sub_id) = result
                        .get()
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .parse::<usize>()
                    else {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Subscription id={result} chain={chain_id:?} is not a valid number",
                        );
                        return;
                    };

                    tracing::trace!(target: LOG_TARGET, "Received subscription id={sub_id} chain={chain_id:?}");

                    let (sub_id_sender, active_subscription) = pending_subscription.into_parts();
                    if sub_id_sender.send(Ok(result)).is_err() {
                        tracing::warn!(
                            target: LOG_TARGET,
                            "Cannot send method response to id={id} chain={chain_id:?}",
                        );

                        return;
                    }

                    // Track this subscription ID if send is successful.
                    data.subscriptions.insert(sub_id, active_subscription);
                } else {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Response id={id} chain={chain_id:?} is not tracked",
                    );
                }
            }
            Ok(RpcResponse::Subscription { method, id, result }) => {
                let Ok(id) = id.parse::<usize>() else {
                    tracing::warn!(target: LOG_TARGET, "Cannot send subscription. Id={id} chain={chain_id:?} is not a valid number");
                    return;
                };

                let Some(subscription_state) = data.subscriptions.get_mut(&id) else {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Subscription response id={id} chain={chain_id:?} method={method} is not tracked",
                    );
                    return;
                };
                if subscription_state.sender.send(result).is_ok() {
                    // Nothing else to do, user is informed about the notification.
                    return;
                }

                // User dropped the receiver, unsubscribe from the method and remove internal tracking.
                let Some(subscription_state) = data.subscriptions.remove(&id) else {
                    // State is checked to be some above, so this should never happen.
                    return;
                };
                // Make a call to unsubscribe from this method.
                let unsub_id = data.next_id();
                let request = format!(
                    r#"{{"jsonrpc":"2.0","id":"{}", "method":"{}","params":["{}"]}}"#,
                    unsub_id, subscription_state.unsubscribe_method, id
                );

                if let Err(err) = self.client.json_rpc_request(request, chain_id) {
                    tracing::warn!(
                        target: LOG_TARGET,
                        "Failed to unsubscribe id={id:?} chain={chain_id:?} method={:?} err={err:?}", subscription_state.unsubscribe_method
                    );
                } else {
                    tracing::debug!(target: LOG_TARGET,"Unsubscribe id={id:?} chain={chain_id:?} method={:?}", subscription_state.unsubscribe_method);
                }
            }
            Err(err) => {
                tracing::warn!(target: LOG_TARGET, "cannot decode RPC response {:?}", err);
            }
        }
    }

    /// Perform the main background task:
    /// - receiving requests from subxt RPC method / subscriptions
    /// - provides the results from the light client back to users.
    pub async fn start_task(
        &mut self,
        from_subxt: mpsc::UnboundedReceiver<FromSubxt>,
        from_node: Vec<AddedChain>,
    ) {
        let from_subxt_event = tokio_stream::wrappers::UnboundedReceiverStream::new(from_subxt);

        let from_node = from_node.into_iter().map(|rpc| {
            Box::pin(futures::stream::unfold(rpc, |mut rpc| async move {
                let response = rpc.rpc_responses.next().await;
                Some(((response, rpc.chain_id), rpc))
            }))
        });
        let stream_combinator = futures::stream::select_all(from_node);

        tokio::pin!(from_subxt_event, stream_combinator);

        let mut from_subxt_event_fut = from_subxt_event.next();
        let mut from_node_event_fut = stream_combinator.next();

        loop {
            match future::select(from_subxt_event_fut, from_node_event_fut).await {
                // Message received from subxt.
                Either::Left((subxt_message, previous_fut)) => {
                    let Some(message) = subxt_message else {
                        tracing::trace!(target: LOG_TARGET, "Subxt channel closed");
                        break;
                    };
                    tracing::trace!(
                        target: LOG_TARGET,
                        "Received register message {:?}",
                        message
                    );

                    self.handle_requests(message).await;

                    from_subxt_event_fut = from_subxt_event.next();
                    from_node_event_fut = previous_fut;
                }
                // Message received from rpc handler: lightclient response.
                Either::Right((node_message, previous_fut)) => {
                    let Some((node_message, chain)) = node_message else {
                        tracing::trace!(target: LOG_TARGET, "Smoldot closed all RPC channels");
                        break;
                    };
                    // Smoldot returns `None` if the chain has been removed (which subxt does not remove).
                    let Some(response) = node_message else {
                        tracing::trace!(target: LOG_TARGET, "Smoldot RPC responses channel closed");
                        break;
                    };
                    tracing::trace!(
                        target: LOG_TARGET,
                        "Received smoldot RPC chain {:?} result {:?}",
                        chain, response
                    );

                    self.handle_rpc_response(chain, response);

                    // Advance backend, save frontend.
                    from_subxt_event_fut = previous_fut;
                    from_node_event_fut = stream_combinator.next();
                }
            }
        }

        tracing::trace!(target: LOG_TARGET, "Task closed");
    }
}

/// The RPC response from the light-client.
/// This can either be a response of a method, or a notification from a subscription.
#[derive(Debug, Clone)]
enum RpcResponse {
    Method {
        /// Response ID.
        id: String,
        /// The result of the method call.
        result: Box<RawValue>,
    },
    Subscription {
        /// RPC method that generated the notification.
        method: String,
        /// Subscription ID.
        id: String,
        /// Result.
        result: Box<RawValue>,
    },
    Error {
        /// Response ID.
        id: String,
        /// Error.
        error: Box<RawValue>,
    },
}

impl std::str::FromStr for RpcResponse {
    type Err = serde_json::Error;

    fn from_str(response: &str) -> Result<Self, Self::Err> {
        // Helper structures to deserialize from raw RPC strings.
        #[derive(Deserialize, Debug)]
        struct Response {
            /// JSON-RPC version.
            #[allow(unused)]
            jsonrpc: String,
            /// Result.
            result: Box<RawValue>,
            /// Request ID
            id: String,
        }
        #[derive(Deserialize)]
        struct NotificationParams {
            /// The ID of the subscription.
            subscription: String,
            /// Result.
            result: Box<RawValue>,
        }
        #[derive(Deserialize)]
        struct ResponseNotification {
            /// JSON-RPC version.
            #[allow(unused)]
            jsonrpc: String,
            /// RPC method that generated the notification.
            method: String,
            /// Result.
            params: NotificationParams,
        }
        #[derive(Deserialize)]
        struct ErrorResponse {
            /// JSON-RPC version.
            #[allow(unused)]
            jsonrpc: String,
            /// Request ID.
            id: String,
            /// Error.
            error: Box<RawValue>,
        }

        // Check if the response can be mapped as an RPC method response.
        let result: Result<Response, _> = serde_json::from_str(response);
        if let Ok(response) = result {
            return Ok(RpcResponse::Method {
                id: response.id,
                result: response.result,
            });
        }

        let result: Result<ResponseNotification, _> = serde_json::from_str(response);
        if let Ok(notification) = result {
            return Ok(RpcResponse::Subscription {
                id: notification.params.subscription,
                method: notification.method,
                result: notification.params.result,
            });
        }

        let error: ErrorResponse = serde_json::from_str(response)?;
        Ok(RpcResponse::Error {
            id: error.id,
            error: error.error,
        })
    }
}
