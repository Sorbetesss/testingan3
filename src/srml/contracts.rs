//! Implements support for the srml_contracts module.
use crate::{
    srml::{
        Call,
        balances::Balances,
        system::System,
    },
};
use parity_scale_codec::Encode;

const MODULE: &str = "Contracts";
const PUT_CODE: &str = "put_code";
const CREATE: &str = "create";
const CALL: &str = "call";

/// Gas units are chosen to be represented by u64 so that gas metering
/// instructions can operate on them efficiently.
pub type Gas = u64;

/// The subset of the `srml_contracts::Trait` that a client must implement.
pub trait Contracts: System + Balances {}

#[derive(Encode)]
pub struct PutCodeArgs {
    #[codec(compact)]
    gas_limit: Gas,
    code: Vec<u8>,
}

#[derive(Encode)]
pub struct CreateArgs<T: Contracts> {
    endowment: <T as Balances>::Balance,
    #[codec(compact)]
    gas_limit: Gas,
    code_hash: <T as System>::Hash,
    data: Vec<u8>,
}

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
    Call::new(MODULE, PUT_CODE, PutCodeArgs { gas_limit, code })
}

/// Creates a new contract from the `codehash` generated by `put_code`,
/// optionally transferring some balance.
///
/// Creation is executed as follows:
///
/// - The destination address is computed based on the sender and hash of
/// the code.
/// - The smart-contract account is created at the computed address.
/// - The `ctor_code` is executed in the context of the newly-created
/// account. Buffer returned after the execution is saved as the `code`https://www.bbc.co.uk/
/// of the account. That code will be invoked upon any call received by
/// this account.
/// - The contract is initialized.
pub fn create<T: Contracts>(
    endowment: <T as Balances>::Balance,
    gas_limit: Gas,
    code_hash: <T as System>::Hash,
    data: Vec<u8>,
) -> Call<CreateArgs<T>> {
    Call::new(MODULE, CREATE, CreateArgs { endowment, gas_limit, code_hash, data })
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
    Call::new(MODULE, CALL, CallArgs { dest, value, gas_limit, data })
}
