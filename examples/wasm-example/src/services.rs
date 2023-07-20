use anyhow::anyhow;
use futures::StreamExt;
use js_sys::Promise;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write;
use subxt::ext::codec::Encode;
use subxt::tx::PartialExtrinsic;
use subxt::{self, OnlineClient, PolkadotConfig};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use yew::{AttrValue, Callback};

#[subxt::subxt(runtime_metadata_path = "../../artifacts/polkadot_metadata_small.scale")]
pub mod polkadot {}

pub(crate) async fn fetch_constant_block_length() -> Result<String, subxt::Error> {
    let api = OnlineClient::<PolkadotConfig>::new().await?;
    let constant_query = polkadot::constants().system().block_length();

    let value = api.constants().at(&constant_query)?;
    Ok(format!("{value:?}"))
}

pub(crate) async fn fetch_events_dynamically() -> Result<Vec<String>, subxt::Error> {
    let api = OnlineClient::<PolkadotConfig>::new().await?;
    let events = api.events().at_latest().await?;
    let mut event_strings = Vec::<String>::new();
    for event in events.iter() {
        let event = event?;
        let pallet = event.pallet_name();
        let variant = event.variant_name();
        let field_values = event.field_values()?;
        event_strings.push(format!("{pallet}::{variant}: {field_values}"));
    }
    Ok(event_strings)
}

/// subscribes to finalized blocks. When a block is received, it is formatted as a string and sent via the callback.
pub(crate) async fn subscribe_to_finalized_blocks(
    cb: Callback<AttrValue>,
) -> Result<(), subxt::Error> {
    let api = OnlineClient::<PolkadotConfig>::new().await?;
    // Subscribe to all finalized blocks:
    let mut blocks_sub = api.blocks().subscribe_finalized().await?;
    while let Some(block) = blocks_sub.next().await {
        let block = block?;
        let mut output = String::new();
        writeln!(output, "Block #{}:", block.header().number).ok();
        writeln!(output, "  Hash: {}", block.hash()).ok();
        writeln!(output, "  Extrinsics:").ok();
        let body = block.body().await?;
        for ext in body.extrinsics().iter() {
            let ext = ext?;
            let idx = ext.index();
            let events = ext.events().await?;
            let bytes_hex = format!("0x{}", hex::encode(ext.bytes()));

            // See the API docs for more ways to decode extrinsics:
            let decoded_ext = ext.as_root_extrinsic::<polkadot::Call>();

            writeln!(output, "    Extrinsic #{idx}:").ok();
            writeln!(output, "      Bytes: {bytes_hex}").ok();
            writeln!(output, "      Decoded: {decoded_ext:?}").ok();
            writeln!(output, "      Events:").ok();

            for evt in events.iter() {
                let evt = evt?;

                let pallet_name = evt.pallet_name();
                let event_name = evt.variant_name();
                let event_values = evt.field_values()?;

                writeln!(output, "        {pallet_name}_{event_name}").ok();
                writeln!(output, "          {}", event_values).ok();
            }
        }
        cb.emit(output.into())
    }
    Ok(())
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = getAccounts)]
    pub fn js_get_accounts() -> Promise;
    #[wasm_bindgen(js_name = signPayload)]
    pub fn js_sign_payload(payload: String, source: String, address: String) -> Promise;
}

/// DTO to communicate with JavaScript
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    /// account name
    pub name: String,
    /// name of the browser extension
    pub source: String,
    pub ty: String,
    /// ss58 formatted address as string. Can be converted into AccountId32 via it's FromStr implementation.
    pub address: String,
}

pub async fn get_accounts() -> Result<Vec<Account>, anyhow::Error> {
    let result = JsFuture::from(js_get_accounts())
        .await
        .map_err(|js_err| anyhow!("{js_err:?}"))?;
    let accounts_str = result
        .as_string()
        .ok_or(anyhow!("Error converting JsValue into String"))?;
    let accounts: Vec<Account> = serde_json::from_str(&accounts_str)?;
    Ok(accounts)
}

fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(bytes.as_ref()))
}

fn encode_to_hex<E: Encode>(input: &E) -> String {
    format!("0x{}", hex::encode(input.encode()))
}

/// this is used because numeric types (e.g. u32) are encoded as little-endian via scale (e.g. 9430 -> d6240000)
/// while we need a big-endian representation for the json (e.g. 9430 -> 000024d6).
fn encode_to_hex_reverse<E: Encode>(input: &E) -> String {
    let mut bytes = input.encode();
    bytes.reverse();
    format!("0x{}", hex::encode(bytes))
}

pub async fn extension_signature_for_partial_extrinsic(
    partial_extrinsic: &PartialExtrinsic<PolkadotConfig, OnlineClient<PolkadotConfig>>,
    api: &OnlineClient<PolkadotConfig>,
    account_source: String,
    account_address: String,
) -> Result<Vec<u8>, anyhow::Error> {
    let params = &partial_extrinsic.additional_and_extra_params;

    let spec_version = encode_to_hex_reverse(&params.spec_version);
    let transaction_version = encode_to_hex_reverse(&params.transaction_version);
    let mortality_checkpoint = encode_to_hex(&params.mortality_checkpoint);
    let era = encode_to_hex(&params.era); // polkadot-js does not seem to accept mortal eras encoded like this
    let genesis_hash = encode_to_hex(&params.genesis_hash);
    let method = to_hex(partial_extrinsic.call_data());
    let nonce = encode_to_hex_reverse(&params.nonce);
    let signed_extensions: Vec<String> = api
        .metadata()
        .extrinsic()
        .signed_extensions()
        .iter()
        .map(|e| e.identifier().to_string())
        .collect();
    let tip = encode_to_hex(&params.tip);

    let payload = json!({
        "specVersion": spec_version,
        "transactionVersion": transaction_version,
        "address": account_address,
        "blockHash": mortality_checkpoint,
        "blockNumber": "0x00000000",
        "era": era,
        "genesisHash": genesis_hash,
        "method": method,
        "nonce": nonce,
        "signedExtensions": signed_extensions,
        "tip": tip,
        "version": 4,
    });

    let payload = payload.to_string();
    let result = JsFuture::from(js_sign_payload(payload, account_source, account_address))
        .await
        .map_err(|js_err| anyhow!("{js_err:?}"))?;
    let signature = result
        .as_string()
        .ok_or(anyhow!("Error converting JsValue into String"))?;
    let signature = hex::decode(&signature[2..])?;
    Ok(signature)
}
