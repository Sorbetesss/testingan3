// Copyright 2019-2020 Parity Technologies (UK) Ltd.
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

//! A library to **sub**mit e**xt**rinsics to a
//! [substrate](https://github.com/paritytech/substrate) node via RPC.

use crate::{
    extra::SignedExtra,
    frame::system::System,
    Encoded,
};
use codec::Encode;
use sp_core::Pair;
use sp_runtime::{
    generic::{
        SignedPayload,
        UncheckedExtrinsic,
    },
    traits::{
        IdentifyAccount,
        Verify,
    },
};
use std::marker::PhantomData;

/// Extrinsic signer.
pub trait Signer<T: System, S: Encode, E: SignedExtra<T>> {
    /// Returns the account id.
    fn account_id(&self) -> &T::AccountId;

    /// Optionally returns a nonce.
    fn nonce(&self) -> Option<T::Index>;

    /// Takes an unsigned extrinsic and returns a signed extrinsic.
    fn sign(
        &self,
        extrinsic: SignedPayload<Encoded, E::Extra>,
    ) -> UncheckedExtrinsic<T::Address, Encoded, S, E::Extra>;
}

/// Extrinsic signer using a private key.
pub struct PairSigner<T: System, S: Encode, E: SignedExtra<T>, P: Pair> {
    _marker: PhantomData<(S, E)>,
    account_id: T::AccountId,
    nonce: Option<T::Index>,
    signer: P,
}

impl<T, S, E, P> PairSigner<T, S, E, P>
where
    T: System,
    S: Encode + Verify + From<P::Signature>,
    S::Signer: From<P::Public> + IdentifyAccount<AccountId = T::AccountId>,
    E: SignedExtra<T>,
    P: Pair,
{
    /// Creates a new `Signer` from a `Pair`.
    pub fn new(signer: P) -> Self {
        let account_id = S::Signer::from(signer.public()).into_account();
        Self {
            _marker: PhantomData,
            account_id,
            nonce: None,
            signer,
        }
    }

    /// Sets the nonce to a new value.
    pub fn set_nonce(&mut self, nonce: T::Index) {
        self.nonce = Some(nonce);
    }

    /// Increment the nonce
    pub fn increment_nonce(&mut self) {
        self.nonce = self.nonce.map(|nonce| nonce + 1.into());
    }
}

impl<T, S, E, P> Signer<T, S, E> for PairSigner<T, S, E, P>
where
    T: System,
    T::AccountId: Into<T::Address>,
    S: Encode,
    E: SignedExtra<T>,
    P: Pair,
    P::Signature: Into<S>,
{
    fn account_id(&self) -> &T::AccountId {
        &self.account_id
    }

    fn nonce(&self) -> Option<T::Index> {
        self.nonce
    }

    fn sign(
        &self,
        extrinsic: SignedPayload<Encoded, E::Extra>,
    ) -> UncheckedExtrinsic<T::Address, Encoded, S, E::Extra> {
        let signature = extrinsic.using_encoded(|payload| self.signer.sign(payload));
        let (call, extra, _) = extrinsic.deconstruct();
        UncheckedExtrinsic::new_signed(
            call,
            self.account_id.clone().into(),
            signature.into(),
            extra,
        )
    }
}
