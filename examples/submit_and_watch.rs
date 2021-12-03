// Copyright 2019-2021 Parity Technologies (UK) Ltd.
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

//! To run this example, a local polkadot node should be running.
//!
//! E.g.
//! ```bash
//! curl "https://github.com/paritytech/polkadot/releases/download/v0.9.11/polkadot" --output /usr/local/bin/polkadot --location
//! polkadot --dev --tmp
//! ```

use sp_keyring::AccountKeyring;
use subxt::{
    ClientBuilder,
    PairSigner,
};

#[subxt::subxt(runtime_metadata_path = "examples/polkadot_metadata.scale")]
pub mod polkadot {}

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let signer = PairSigner::new(AccountKeyring::Alice.pair());
    let dest = AccountKeyring::Bob.to_account_id().into();

    let api = ClientBuilder::new()
        .build()
        .await?
        .to_runtime_api::<polkadot::RuntimeApi<polkadot::DefaultConfig>>();

    let balance_transfer = api
        .tx()
        .balances();

    let transaction_progress = balance_transfer
        .transfer(dest, 10_000)
        .sign_and_submit_then_watch(&signer)
        .await?;

    let transaction_events = transaction_progress
        .wait_for_finalized()
        .await?
        .events()
        .await?;

    let transfer_event = transaction_events
        .filter_map(|e| e.as_event::<polkadot::balances::events::Transfer>())
        .next();

    if let Some(event) = transfer_event {
        println!("Balance transfer success: value: {:?}", event.2);
    } else {
        println!("Failed to find Balances::Transfer Event");
    }
    Ok(())
}
