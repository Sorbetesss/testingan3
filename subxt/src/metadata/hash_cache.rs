// Copyright 2019-2022 Parity Technologies (UK) Ltd.
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

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::RwLock,
};

/// A cache with the simple goal of storing 32 byte hashes against pallet+item keys
#[derive(Default, Debug)]
pub struct HashCache {
    inner: RwLock<HashMap<PalletItemKey<'static>, [u8; 32]>>,
}

impl HashCache {
    /// get a hash out of the cache by its pallet and item key. If the item doesn't exist,
    /// run the function provided to obtain a hash to insert (or bail with some error on failure).
    pub fn get_or_insert<F, E>(&self, pallet: &str, item: &str, f: F) -> Result<[u8; 32], E>
    where F: FnOnce() -> Result<[u8; 32], E>
    {
        let maybe_hash = self.inner
            .read()
            .unwrap()
            .get(&PalletItemKey::new(pallet, item))
            .map(|i| *i);

        if let Some(hash) = maybe_hash {
            return Ok(hash)
        }

        let hash = f()?;
        self.inner
            .write()
            .unwrap()
            .insert(PalletItemKey::new(pallet.to_string(), item.to_string()), hash);

        Ok(hash)
    }
}

/// This exists so that we can look items up in the cache using &strs, without having to allocate
/// Strings first (as you'd have to do to construct something like an `&(String,String)` key).
#[derive(Debug, PartialEq, Eq, Hash)]
struct PalletItemKey<'a> {
    pallet: Cow<'a, str>,
    item: Cow<'a, str>,
}

impl<'a> PalletItemKey<'a> {
    fn new(pallet: impl Into<Cow<'a, str>>, item: impl Into<Cow<'a, str>>) -> Self {
        PalletItemKey {
            pallet: pallet.into(),
            item: item.into(),
        }
    }
}
