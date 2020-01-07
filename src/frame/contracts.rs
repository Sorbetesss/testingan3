// Copyright 2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-subxt.
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
// along with substrate-subxt.  If not, see <http://www.gnu.org/licenses/>.

//! Implements support for the pallet_contracts module.

use crate::frame::{
    balances::Balances,
    system::System,
    Call,
};
use codec::Encode;

const MODULE: &str = "Contracts";

mod calls {
    pub const PUT_CODE: &str = "put_code";
    pub const INSTANTIATE: &str = "instantiate";
    pub const CALL: &str = "call";
}

#[allow(unused)]
mod events {
    pub const CODE_STORED: &str = "CodeStored";
    pub const INSTANTIATED: &str = "Instantiated";
}

/// Gas units are chosen to be represented by u64 so that gas metering
/// instructions can operate on them efficiently.
pub type Gas = u64;

/// The subset of the `pallet_contracts::Trait` that a client must implement.
pub trait Contracts: System + Balances {}

/// Arguments for uploading contract code to the chain
#[derive(Encode)]
pub struct PutCodeArgs {
    #[codec(compact)]
    gas_limit: Gas,
    code: Vec<u8>,
}

/// Arguments for creating an instance of a contract
#[derive(Encode)]
pub struct InstantiateArgs<T: Contracts> {
    #[codec(compact)]
    endowment: <T as Balances>::Balance,
    #[codec(compact)]
    gas_limit: Gas,
    code_hash: <T as System>::Hash,
    data: Vec<u8>,
}

/// Arguments for calling a contract
#[derive(Encode)]
pub struct CallArgs<T: Contracts> {
    dest: <T as System>::Address,
    value: <T as Balances>::Balance,
    #[codec(compact)]
    gas_limit: Gas,
    data: Vec<u8>,
}

/// Stores the given binary Wasm code into the chain's storage and returns
/// its `codehash`.
/// You can instantiate contracts only with stored code.
pub fn put_code(gas_limit: Gas, code: Vec<u8>) -> Call<PutCodeArgs> {
    Call::new(MODULE, calls::PUT_CODE, PutCodeArgs { gas_limit, code })
}

/// Creates a new contract from the `codehash` generated by `put_code`,
/// optionally transferring some balance.
///
/// Creation is executed as follows:
///
/// - The destination address is computed based on the sender and hash of
/// the code.
/// - The smart-contract account is instantiated at the computed address.
/// - The `ctor_code` is executed in the context of the newly-instantiated
/// account. Buffer returned after the execution is saved as the `code`https://www.bbc.co.uk/
/// of the account. That code will be invoked upon any call received by
/// this account.
/// - The contract is initialized.
pub fn instantiate<T: Contracts>(
    endowment: <T as Balances>::Balance,
    gas_limit: Gas,
    code_hash: <T as System>::Hash,
    data: Vec<u8>,
) -> Call<InstantiateArgs<T>> {
    Call::new(
        MODULE,
        calls::INSTANTIATE,
        InstantiateArgs {
            endowment,
            gas_limit,
            code_hash,
            data,
        },
    )
}

/// Makes a call to an account, optionally transferring some balance.
///
/// * If the account is a smart-contract account, the associated code will
///  be executed and any value will be transferred.
/// * If the account is a regular account, any value will be transferred.
/// * If no account exists and the call value is not less than
/// `existential_deposit`, a regular account will be created and any value
///  will be transferred.
pub fn call<T: Contracts>(
    dest: <T as System>::Address,
    value: <T as Balances>::Balance,
    gas_limit: Gas,
    data: Vec<u8>,
) -> Call<CallArgs<T>> {
    Call::new(
        MODULE,
        calls::CALL,
        CallArgs {
            dest,
            value,
            gas_limit,
            data,
        },
    )
}

#[cfg(test)]
mod tests {
    use codec::{
        Codec,
        Error as CodecError,
    };
    use futures::Future;
    use sp_core::Pair;
    use sp_keyring::AccountKeyring;
    use sp_runtime::traits::{
        IdentifyAccount,
        Verify,
    };

    use super::events;
    use crate::{
        frame::contracts::MODULE,
        tests::test_setup,
        Balances,
        Client,
        DefaultNodeRuntime as Runtime,
        Error,
        System,
    };

    type AccountId = <Runtime as System>::AccountId;

    fn put_code<T, P, S>(
        client: &Client<T, S>,
        signer: P,
    ) -> impl Future<Item = Option<Result<T::Hash, CodecError>>, Error = Error>
    where
        T: System + Balances + Send + Sync,
        T::Address: From<T::AccountId>,
        P: Pair,
        P::Signature: Codec,
        S: 'static,
        S: Verify + Codec + From<P::Signature>,
        S::Signer: From<P::Public> + IdentifyAccount<AccountId = T::AccountId>,
    {
        const CONTRACT: &str = r#"
(module
    (func (export "call"))
    (func (export "deploy"))
)
"#;
        let wasm = wabt::wat2wasm(CONTRACT).expect("invalid wabt");

        client.xt(signer, None).and_then(|xt| {
            xt.watch()
                .submit(super::put_code(500_000, wasm))
                .map(|result| result.find_event::<T::Hash>(MODULE, events::CODE_STORED))
        })
    }

    #[test]
    #[ignore] // requires locally running substrate node
    fn tx_put_code() {
        let (mut rt, client) = test_setup();

        let signer = AccountKeyring::Alice.pair();
        let code_hash = rt.block_on(put_code(&client, signer)).unwrap();

        assert!(
            code_hash.is_some(),
            "Contracts CodeStored event should be present"
        );
        assert!(
            code_hash.unwrap().is_ok(),
            "CodeStored Hash should decode successfully"
        );
    }

    #[test]
    #[ignore] // requires locally running substrate node
    fn tx_instantiate() {
        let (mut rt, client) = test_setup();

        let signer = AccountKeyring::Alice.pair();
        let code_hash = rt
            .block_on(put_code(&client, signer.clone()))
            .unwrap()
            .unwrap()
            .unwrap();

        println!("{:?}", code_hash);

        let instantiate = client.xt(signer, None).and_then(move |xt| {
            xt.watch()
                .submit(super::instantiate::<Runtime>(
                    100_000_000_000_000,
                    500_000,
                    code_hash,
                    Vec::new(),
                ))
                .map(|result| {
                    result.find_event::<(AccountId, AccountId)>(
                        MODULE,
                        events::INSTANTIATED,
                    )
                })
        });

        let result = rt.block_on(instantiate).unwrap();

        assert!(
            result.is_some(),
            "Contracts Instantiated event should be present"
        );
        assert!(
            result.unwrap().is_ok(),
            "Instantiated Event should decode successfully"
        );
    }
}
