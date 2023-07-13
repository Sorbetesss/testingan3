use std::fmt::Debug;
use anyhow::anyhow;
use futures::FutureExt;
use serde::Deserialize;
use subxt::{OnlineClient, PolkadotConfig};
use subxt::dynamic::Value;
use subxt::ext::codec::{Decode, Encode};
use subxt::tx::SubmittableExtrinsic;
use subxt::utils::{AccountId32, MultiSignature};


use web_sys::{HtmlInputElement};
use yew::prelude::*;
use crate::services::{Account, get_accounts, polkadot, sign_hex_message, sign_payload, SignerPayloadForJS};

pub struct SigningExamplesComponent {
    message: String,
    online_client: Option<OnlineClient<PolkadotConfig>>,
    stage: SigningStage,
}

pub enum SigningStage {
    Error(String),
    CreatingOnlineClient,
    EnterMessage,
    RequestingAccounts,
    SelectAccount(Vec<Account>),
    Signing(Account),
    SigningSuccess {
        signer_account: Account,
        signature: String,
        signed_extrinsic_hex: String,
        submitting_stage: SubmittingStage,
    },
}

pub enum SubmittingStage {
    Initial {
        signed_extrinsic: SubmittableExtrinsic<PolkadotConfig, OnlineClient<PolkadotConfig>>,

    },
    Submitting,
    Success {
        remark_event: polkadot::system::events::ExtrinsicSuccess
    },
    Error(anyhow::Error),
}


pub enum Message {
    Error(anyhow::Error),
    OnlineClientCreated(OnlineClient<PolkadotConfig>),
    ChangeMessage(String),
    RequestAccounts,
    ReceivedAccounts(Vec<Account>),
    /// usize represents account index in Vec<Account>
    SignWithAccount(usize),
    ReceivedSignature(String, SubmittableExtrinsic<PolkadotConfig, OnlineClient<PolkadotConfig>>),
    SubmitSigned,
    ExtrinsicFinalized {
        remark_event: polkadot::system::events::ExtrinsicSuccess
    },
    ExtrinsicFailed(anyhow::Error),
}

impl Component for SigningExamplesComponent {
    type Message = Message;

    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        ctx.link().send_future(OnlineClient::<PolkadotConfig>::new().map(|res| {
            match res {
                Ok(online_client) => Message::OnlineClientCreated(online_client),
                Err(err) => Message::Error(anyhow!("Online Client could not be created. Make sure you have a local node running:\n{err}")),
            }
        }));
        SigningExamplesComponent {
            message: "Hello".to_string(),
            stage: SigningStage::CreatingOnlineClient,
            online_client: None,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Message::OnlineClientCreated(online_client) => {
                self.online_client = Some(online_client);
                self.stage = SigningStage::EnterMessage;
            }
            Message::ChangeMessage(message) => {
                web_sys::console::log_1(&message.clone().into());
                self.message = message
            }
            Message::RequestAccounts => {
                self.stage = SigningStage::RequestingAccounts;
                ctx.link().send_future(get_accounts().map(
                    |accounts_or_err| match accounts_or_err {
                        Ok(accounts) => Message::ReceivedAccounts(accounts),
                        Err(err) => Message::Error(err),
                    },
                ));
            }
            Message::ReceivedAccounts(accounts) => {
                self.stage = SigningStage::SelectAccount(accounts);
            }
            Message::Error(err) => self.stage = SigningStage::Error(err.to_string()),
            Message::SignWithAccount(i) => {
                if let SigningStage::SelectAccount(accounts) = &self.stage {
                    let account = accounts.get(i).unwrap();
                    let account_address = account.address.clone();
                    let account_source = account.source.clone();
                    let account_id: AccountId32 = account_address.parse().unwrap();
                    web_sys::console::log_1(&account_id.to_string().into());

                    self.stage = SigningStage::Signing(account.clone());

                    let remark_call = polkadot::tx().system().remark(self.message.as_bytes().to_vec());
                    let api = self.online_client.as_ref().unwrap().clone();

                    ctx.link()
                        .send_future(
                            async move {
                                // let polkadot_js_signed = {
                                //     let signed_extrinsic_hex = "0xb9018400d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d014e6dc8f9439b93886e88e83e5d6f7f8b67f8411ca37b79e8c68fe0159520cf1a3d9efce3baf75726a1bfab50c01e5dbccf82dcce6a6646b68e83e6605795028500000000001448656c6c6f";
                                //     let tx_bytes = hex::decode(&signed_extrinsic_hex[2..]).unwrap();
                                //     SubmittableExtrinsic::from_bytes(api.clone(), tx_bytes)
                                // };
                                // let dry_res = polkadot_js_signed.dry_run(None).await;
                                // web_sys::console::log_1(&format!("polkadot_js_signed dry res: {:?}", dry_res).into());
                                // Message::ReceivedSignature("signature".into(), polkadot_js_signed)

                                let partial_extrinsic =
                                    match api.tx().create_partial_signed(&remark_call, &account_id, Default::default()).await {
                                        Ok(partial_extrinsic) => partial_extrinsic,
                                        Err(err) => {
                                            return Message::Error(anyhow!("could not create partial extrinsic:\n{:?}", err));
                                        }
                                    };



                                fn to_hex(bytes: impl AsRef<[u8]>) -> String {
                                    format!("0x{}", hex::encode(bytes.as_ref()))
                                }

                                fn encode_to_hex<E: Encode>(input: &E) -> String {
                                    format!("0x{}", hex::encode(input.encode()))
                                }

                                fn encode_to_hex_reverse<E: Encode>(input: &E) -> String {
                                    let mut bytes = input.encode();
                                    bytes.reverse();
                                    format!("0x{}", hex::encode(bytes))
                                }


                                // check the payload (debug)
                                let params = &partial_extrinsic.additional_and_extra_params;
                                // web_sys::console::log_1(&format!("params.genesis_hash: {:?}", params.genesis_hash).into());
                                // web_sys::console::log_1(&format!("params.era: {:?}", params.era).into());
                                //
                                // web_sys::console::log_1(&format!("spec_version: {:?} {:?}", &partial_extrinsic.additional_and_extra_params.spec_version, encode_to_hex_reverse(&partial_extrinsic.additional_and_extra_params.spec_version)).into());

                                let js_payload = SignerPayloadForJS {
                                    spec_version: encode_to_hex_reverse(&params.spec_version),
                                    transaction_version: encode_to_hex_reverse(&partial_extrinsic.additional_and_extra_params.transaction_version),
                                    address: account_address.clone(),
                                    block_hash: encode_to_hex(&params.genesis_hash),
                                    block_number: "0x00000000".into(), // immortal
                                    era: "0x0000".into(), // immortal
                                    genesis_hash: encode_to_hex(&params.genesis_hash),
                                    method: to_hex(partial_extrinsic.call_data()),
                                    nonce: encode_to_hex_reverse(&params.nonce),
                                    signed_extensions: vec![
                                        "CheckNonZeroSender",
                                        "CheckSpecVersion",
                                        "CheckTxVersion",
                                        "CheckGenesis",
                                        "CheckMortality",
                                        "CheckNonce",
                                        "CheckWeight",
                                        "ChargeTransactionPayment",
                                        "PrevalidateAttests",
                                    ].into_iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                                    tip: "0x00000000000000000000000000000000".into(),
                                    version: 4,
                                };

                                let Ok(signature) = sign_payload(js_payload, account_source, account_address).await else {
                                    return Message::Error(anyhow!("Signing via extension failed"));
                                };
                                web_sys::console::log_1(&format!("signature: {}", &signature).into());

                                // these are 65 bytes: 01 for the Sr25519 variant and 64 bytes after that.
                                let signature_bytes = hex::decode(&signature[2..]).unwrap();
                                // the deafult for the polkadot
                                let Ok(multi_signature) = MultiSignature::decode(&mut &signature_bytes[..]) else {
                                    return Message::Error(anyhow!("MultiSignature Decoding"));
                                };
                                web_sys::console::log_1(&format!("multi_signature: {:?}", &multi_signature).into());


                                let signed_extrinsic = partial_extrinsic.sign_with_address_and_signature(&account_id.into(), &multi_signature);
                                web_sys::console::log_1(&format!("signed_extrinsic hex: {:?}", hex::encode(signed_extrinsic.encoded())).into());
                                let dry_res = signed_extrinsic.dry_run(None).await;
                                web_sys::console::log_1(&format!("dry res: {:?}", dry_res).into());
                                return Message::Error(anyhow!("after signing"));

                                let multi_signature: MultiSignature = MultiSignature::Sr25519(signature_bytes.try_into().unwrap());

                                //
                                // web_sys::console::log_1(&format!("signer payload{:?}", partial_extrinsic.signer_payload()).into());
                                // let hex_extrinsic_to_sign = format!("0x{}", hex::encode(partial_extrinsic.signer_payload()));
                                // web_sys::console::log_1(&hex_extrinsic_to_sign.clone().into());
                                //
                                // // get the signature via browser extension
                                // let Ok(signature) = sign_hex_message(hex_extrinsic_to_sign, account_source, account_address).await else {
                                //     return Message::Error(anyhow!("Signing failed"));
                                // };
                                // let signature_bytes = hex::decode(&signature[2..]).unwrap();
                                // web_sys::console::log_1(&format!("{:?}", signature_bytes).into());
                                // let multi_signature: MultiSignature = MultiSignature::Sr25519(signature_bytes.try_into().unwrap());
                                // let signed_extrinsic = partial_extrinsic.sign_with_address_and_signature(&account_id.into(), &multi_signature);
                                //
                                // // test via dry run (debug)
                                // let dry_res = signed_extrinsic.dry_run(None).await;
                                // web_sys::console::log_1(&format!("dry res: {:?}", dry_res).into());
                                //
                                // // return the signature and signed extrinsic
                                // Message::ReceivedSignature(signature, signed_extrinsic)
                            }
                        );
                }
            }
            Message::ReceivedSignature(signature, signed_extrinsic) => {
                if let SigningStage::Signing(account) = &self.stage {
                    let signed_extrinsic_hex = format!("0x{}", hex::encode(signed_extrinsic.encoded()));
                    self.stage = SigningStage::SigningSuccess {
                        signer_account: account.clone(),
                        signature,
                        signed_extrinsic_hex,
                        submitting_stage: SubmittingStage::Initial { signed_extrinsic },
                    }
                }
            }
            Message::SubmitSigned => {
                if let SigningStage::SigningSuccess { submitting_stage: submitting_stage @ SubmittingStage::Initial { .. }, .. } = &mut self.stage {
                    let SubmittingStage::Initial { signed_extrinsic } = std::mem::replace(submitting_stage, SubmittingStage::Submitting) else {
                        panic!("unreachable")
                    };

                    ctx.link().send_future(async move {
                        match submit_wait_finalized_and_get_remark_event(signed_extrinsic).await {
                            Ok(remark_event) => Message::ExtrinsicFinalized { remark_event },
                            Err(err) => Message::ExtrinsicFailed(err)
                        }
                    });
                }
            }
            Message::ExtrinsicFinalized { remark_event } => {
                if let SigningStage::SigningSuccess { submitting_stage, .. } = &mut self.stage {
                    *submitting_stage = SubmittingStage::Success { remark_event }
                }
            }
            Message::ExtrinsicFailed(err) => {
                if let SigningStage::SigningSuccess { submitting_stage, .. } = &mut self.stage {
                    *submitting_stage = SubmittingStage::Error(err)
                }
            }
        };
        true
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let message_html: Html = match &self.stage {
            SigningStage::Error(_) | SigningStage::EnterMessage | SigningStage::CreatingOnlineClient => html!(<></>),
            _ => {
                let hex_message = format!("0x{}", hex::encode(&self.message));
                html!(
                    <div>
                        <div class="mb">
                            <b>{"Message: "}</b> <br/>
                            {&self.message}
                        </div>
                        <div class="mb">
                            <b>{"Hex representation of message:"}</b> <br/>
                            {hex_message}
                        </div>
                    </div>
                )
            }
        };

        let signer_account_html: Html = match &self.stage {
            SigningStage::Signing(signer_account)
            | SigningStage::SigningSuccess {
                signer_account, ..
            } => {
                html!(
                    <div class="mb">
                            <b>{"Account used for signing: "}</b> <br/>
                            {"Extension: "}{&signer_account.source} <br/>
                            {"Name: "}{&signer_account.name} <br/>
                            {"Address: "}{&signer_account.address} <br/>
                    </div>
                )
            }
            _ => html!(<></>),
        };

        let stage_html: Html = match &self.stage {
            SigningStage::Error(error_message) => {
                html!(<div class="error"> {"Error: "} {error_message} </div>)
            }
            SigningStage::CreatingOnlineClient => {
                html!(
                    <div>
                        <b>{"Creating Online Client..."}</b>
                    </div>
                )
            }
            SigningStage::EnterMessage => {
                let get_accounts_click = ctx.link().callback(|_| Message::RequestAccounts);
                let hex_message = format!("0x{}", hex::encode(&self.message));
                let on_input = ctx.link().callback(move |event: InputEvent| {
                    let input_element = event.target_dyn_into::<HtmlInputElement>().unwrap();
                    let value = input_element.value();
                    Message::ChangeMessage(value)
                });

                html!(
                    <>
                        <div class="mb">{"Enter a message for the \"remark\" call in the \"System\" pallet:"}</div>
                        <input oninput={on_input} class="mb" value={AttrValue::from(self.message.clone())}/>
                        <div class="mb"><b>{"Hex representation of message:"}</b><br/>{hex_message}</div>
                        <button onclick={get_accounts_click}> {"=> Select an Account for Signing"} </button>
                    </>
                )
            }
            SigningStage::RequestingAccounts => {
                html!(<div>{"Querying extensions for accounts..."}</div>)
            }
            SigningStage::SelectAccount(accounts) => {
                if accounts.is_empty() {
                    html!(<div>{"No Web3 extension accounts found. Install Talisman or the Polkadot.js extension and add an account."}</div>)
                } else {
                    html!(
                        <>
                            <div>{"Select an account you want to use for signing:"}</div>
                            { for accounts.iter().enumerate().map(|(i, account)| {
                                let sign_with_account = ctx.link().callback(move |_| Message::SignWithAccount(i));
                                html! {
                                    <button onclick={sign_with_account}>
                                        {&account.source} {" | "} {&account.name}<br/>
                                        <small>{&account.address}</small>
                                    </button>
                                }
                            }) }
                        </>
                    )
                }
            }
            SigningStage::Signing(_) => {
                html!(<div>{"Singing message with browser extension..."}</div>)
            }
            SigningStage::SigningSuccess {
                signature,
                signed_extrinsic_hex,
                submitting_stage,
                ..
            } => {
                let submitting_stage_html = match submitting_stage {
                    SubmittingStage::Initial { .. } => {
                        let submit_extrinsic_click = ctx.link().callback(move |_| Message::SubmitSigned);
                        html!(<button onclick={submit_extrinsic_click}> {"=> Submit the signed extrinsic"} </button>)
                    }
                    SubmittingStage::Submitting => html!(<div> {"Submitting Extrinsic..."}</div>),
                    SubmittingStage::Success { remark_event } => {
                        html!(<div style="overflow-wrap: break-word;"> <b>{"Successfully submitted Extrinsic. Event:"}</b> <br/> {format!("{:?}", remark_event)} </div>)
                    }
                    SubmittingStage::Error(err) => {
                        html!(<div class="error"> {"Error: "} {err.to_string()} </div>)
                    }
                };

                html!(
                    <>
                        <div style="overflow-wrap: break-word;">
                            <b>{"Received signature: "}</b><br/>
                            {signature}
                        </div>
                        <div style="overflow-wrap: break-word;">
                            <b>{"Hex representation of signed extrinsic: "}</b> <br/>
                            {signed_extrinsic_hex}
                        </div>
                        {submitting_stage_html}
                    </>
                )
            }
        };

        html! {
            <div>
                <a href="/"> <button>{"<= Back"}</button></a>
                <h1>{"Subxt Signing Example"}</h1>
                {message_html}
                {signer_account_html}
                {stage_html}
            </div>
        }
    }
}

async fn submit_wait_finalized_and_get_remark_event(extrinsic: SubmittableExtrinsic<PolkadotConfig, OnlineClient<PolkadotConfig>>) -> Result<polkadot::system::events::ExtrinsicSuccess, anyhow::Error> {
    let events = extrinsic.submit_and_watch()
        .await?
        .wait_for_finalized_success()
        .await?;

    let events_str = format!("{:?}", &events);
    web_sys::console::log_1(&events_str.into());
    for event in events.find::<polkadot::system::events::ExtrinsicSuccess>() {
        web_sys::console::log_1(&format!("{:?}", event).into());
    }

    let success = events.find_first::<polkadot::system::events::ExtrinsicSuccess>()?;
    success.ok_or(anyhow!("ExtrinsicSuccess not found in events"))
}

