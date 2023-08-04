// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

use super::{signed_extensions, ExtrinsicParams};
use super::{Config, Header};

/// The default [`super::ExtrinsicParams`] implementation understands common signed extensions
/// and how to apply them to a given chain.
pub type DefaultExtrinsicParams<T> = signed_extensions::AnyOf<
    T,
    (
        signed_extensions::CheckSpecVersion,
        signed_extensions::CheckTxVersion,
        signed_extensions::CheckNonce,
        signed_extensions::CheckGenesis<T>,
        signed_extensions::CheckMortality<T>,
        signed_extensions::ChargeAssetTxPayment,
        signed_extensions::ChargeTransactionPayment,
    ),
>;

/// A builder that outputs the set of [`super::ExtrinsicParams::OtherParams`] required for
/// [`DefaultExtrinsicParams`]. This may expose methods that aren't applicable to the current
/// chain; such values will simply be ignored if so.
pub struct DefaultExtrinsicParamsBuilder<T: Config> {
    mortality_checkpoint_hash: Option<T::Hash>,
    mortality_checkpoint_number: u64,
    mortality_period: u64,
    tip: u128,
    tip_of: u128,
    tip_of_asset_id: Option<u32>,
}

impl<T: Config> DefaultExtrinsicParamsBuilder<T> {
    /// Configure new extrinsic params. We default to providing no tip
    /// and using an immortal transaction unless otherwise configured
    pub fn new() -> Self {
        Self {
            mortality_checkpoint_hash: None,
            mortality_checkpoint_number: 0,
            mortality_period: 0,
            tip: 0,
            tip_of: 0,
            tip_of_asset_id: None,
        }
    }

    /// Make the transaction mortal, given a block header that it should be mortal from,
    /// and the number of blocks (roughly; it'll be rounded to a power of two) that it will
    /// be mortal for.
    pub fn mortal(mut self, from_block: &T::Header, for_n_blocks: u64) -> Self {
        self.mortality_checkpoint_hash = Some(from_block.hash());
        self.mortality_checkpoint_number = from_block.number().into();
        self.mortality_period = for_n_blocks;
        self
    }

    /// Make the transaction mortal, given a block number and block hash (which must both point to
    /// the same block) that it should be mortal from, and the number of blocks (roughly; it'll be
    /// rounded to a power of two) that it will be mortal for.
    ///
    /// Prefer to use [`DefaultExtrinsicParamsBuilder::mortal()`], which ensures that the block hash
    /// and number align.
    pub fn mortal_unchecked(
        mut self,
        from_block_number: u64,
        from_block_hash: T::Hash,
        for_n_blocks: u64,
    ) -> Self {
        self.mortality_checkpoint_hash = Some(from_block_hash);
        self.mortality_checkpoint_number = from_block_number;
        self.mortality_period = for_n_blocks;
        self
    }

    /// Provide a tip to the block author in the chain's native token.
    pub fn tip(mut self, tip: u128) -> Self {
        self.tip = tip;
        self.tip_of = tip;
        self.tip_of_asset_id = None;
        self
    }

    /// Provide a tip to the block auther using the token denominated by the `asset_id` provided. This
    /// is not applicable on chains which don't use the `ChargeAssetTxPayment` signed extension; in this
    /// case, no tip will be given.
    pub fn tip_of(mut self, tip: u128, asset_id: u32) -> Self {
        self.tip = 0;
        self.tip_of = tip;
        self.tip_of_asset_id = Some(asset_id);
        self
    }

    /// Return the "raw" params as required. This doesn't need to be called in normal usage.
    pub fn raw(self) -> OtherParams<T> {
        let check_mortality_params = if let Some(checkpoint_hash) = self.mortality_checkpoint_hash {
            signed_extensions::CheckMortalityParams::mortal(
                self.mortality_period,
                self.mortality_checkpoint_number,
                checkpoint_hash,
            )
        } else {
            signed_extensions::CheckMortalityParams::immortal()
        };

        let charge_asset_tx_params = if let Some(asset_id) = self.tip_of_asset_id {
            signed_extensions::ChargeAssetTxPaymentParams::tip_of(self.tip, asset_id)
        } else {
            signed_extensions::ChargeAssetTxPaymentParams::tip(self.tip)
        };

        let charge_transaction_params =
            signed_extensions::ChargeTransactionPaymentParams::tip(self.tip);

        (
            (),
            (),
            (),
            (),
            check_mortality_params,
            charge_asset_tx_params,
            charge_transaction_params,
        )
    }
}

type OtherParams<T> = (
    (),
    (),
    (),
    (),
    signed_extensions::CheckMortalityParams<T>,
    signed_extensions::ChargeAssetTxPaymentParams,
    signed_extensions::ChargeTransactionPaymentParams,
);

impl<T: Config> From<DefaultExtrinsicParamsBuilder<T>> for OtherParams<T> {
    fn from(v: DefaultExtrinsicParamsBuilder<T>) -> Self {
        v.raw()
    }
}

// We have to manually write out `OtherParams<T>` for some reason to avoid type errors in the `From` impl.
// So, here we ensure that `OtherParams<T>` is equal to `<DefaultExtrinsicParams<T> as ExtrinsicParams<T>>::OtherParams`.
// We'll get a compile error if not.
#[allow(unused)]
fn assert_otherparams_eq() {
    struct Ty<Inner>(Inner);
    fn assert_eq<T: Config>(t: Ty<OtherParams<T>>) {
        match t {
            Ty::<<DefaultExtrinsicParams<T> as ExtrinsicParams<T>>::OtherParams>(_) => {}
        }
    }
}
