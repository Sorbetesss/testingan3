// Copyright 2019-2023 Parity Technologies (UK) Ltd.
// This file is dual-licensed as Apache-2.0 or GPL-3.0.
// see LICENSE for license details.

//! An sr25519 keypair implementation.

use crate::crypto::{seed_from_entropy, DeriveJunction, SecretUri};
use hex::FromHex;
use schnorrkel::{
    derive::{ChainCode, Derivation},
    ExpansionMode, MiniSecretKey,
};
use secrecy::ExposeSecret;

const SEED_LENGTH: usize = schnorrkel::keys::MINI_SECRET_KEY_LENGTH;
const SIGNING_CTX: &[u8] = b"substrate";

/// Seed bytes used to generate a key pair.
pub type Seed = [u8; SEED_LENGTH];

/// A signature generated by [`Keypair::sign()`]. These bytes are equivalent
/// to a Substrate `MultiSignature::sr25519(bytes)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// The public key for an [`Keypair`] key pair. This is equivalent to a
/// Substrate `AccountId32`.
pub struct PublicKey(pub [u8; 32]);

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// An sr25519 keypair implementation. While the API is slightly different, the logic for
/// this has been taken from `sp_core::sr25519` and we test against this to ensure conformity.
#[derive(Debug, Clone)]
pub struct Keypair(schnorrkel::Keypair);

impl Keypair {
    /// Create am sr25519 keypair from a [`SecretUri`]. See the [`SecretUri`] docs for more.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ SecretUri, sr25519::Keypair };
    /// use std::str::FromStr;
    ///
    /// let uri = SecretUri::from_str("//Alice").unwrap();
    /// let keypair = Keypair::from_uri(&uri).unwrap();
    ///
    /// keypair.sign(b"Hello world!");
    /// ```
    pub fn from_uri(uri: &SecretUri) -> Result<Self, Error> {
        let SecretUri {
            junctions,
            phrase,
            password,
        } = uri;

        // If the phrase is hex, convert bytes directly into a seed, ignoring password.
        // Else, parse the phrase string taking the password into account. This is
        // the same approach taken in sp_core::crypto::Pair::from_string_with_seed.
        let key = if let Some(hex_str) = phrase.expose_secret().strip_prefix("0x") {
            let seed = Seed::from_hex(hex_str)?;
            Self::from_seed(seed)?
        } else {
            let phrase = bip39::Mnemonic::parse(phrase.expose_secret().as_str())?;
            let pass_str = password.as_ref().map(|p| p.expose_secret().as_str());
            Self::from_phrase(&phrase, pass_str)?
        };

        // Now, use any "junctions" to derive a new key from this root key.
        Ok(key.derive(junctions.iter().copied()))
    }

    /// Create am sr25519 keypair from a BIP-39 mnemonic phrase and optional password.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ bip39::Mnemonic, sr25519::Keypair };
    ///
    /// let phrase = "bottom drive obey lake curtain smoke basket hold race lonely fit walk";
    /// let mnemonic = Mnemonic::parse(phrase).unwrap();
    /// let keypair = Keypair::from_phrase(&mnemonic, None).unwrap();
    ///
    /// keypair.sign(b"Hello world!");
    /// ```
    pub fn from_phrase(mnemonic: &bip39::Mnemonic, password: Option<&str>) -> Result<Self, Error> {
        let big_seed = seed_from_entropy(&mnemonic.to_entropy(), password.unwrap_or(""))
            .ok_or(Error::InvalidSeed)?;

        let seed: Seed = big_seed[..SEED_LENGTH]
            .try_into()
            .expect("should be valid Seed");

        Self::from_seed(seed)
    }

    /// Turn a 32 byte seed into a keypair.
    ///
    /// # Warning
    ///
    /// This will only be secure if the seed is secure!
    pub fn from_seed(seed: Seed) -> Result<Self, Error> {
        let keypair = MiniSecretKey::from_bytes(&seed)
            .map_err(|_| Error::InvalidSeed)?
            .expand_to_keypair(ExpansionMode::Ed25519);

        Ok(Keypair(keypair))
    }

    /// Derive a child key from this one given a series of junctions.
    ///
    /// # Example
    ///
    /// ```rust
    /// use subxt_signer::{ bip39::Mnemonic, sr25519::Keypair, DeriveJunction };
    ///
    /// let phrase = "bottom drive obey lake curtain smoke basket hold race lonely fit walk";
    /// let mnemonic = Mnemonic::parse(phrase).unwrap();
    /// let keypair = Keypair::from_phrase(&mnemonic, None).unwrap();
    ///
    /// // Equivalent to the URI path '//Alice/stash':
    /// let new_keypair = keypair.derive([
    ///     DeriveJunction::hard("Alice"),
    ///     DeriveJunction::soft("stash")
    /// ]);
    /// ```
    pub fn derive<Js: IntoIterator<Item = DeriveJunction>>(&self, junctions: Js) -> Self {
        let init = self.0.secret.clone();
        let result = junctions.into_iter().fold(init, |acc, j| match j {
            DeriveJunction::Soft(cc) => acc.derived_key_simple(ChainCode(cc), []).0,
            DeriveJunction::Hard(cc) => {
                let seed = acc.hard_derive_mini_secret_key(Some(ChainCode(cc)), b"").0;
                seed.expand(ExpansionMode::Ed25519)
            }
        });
        Self(result.into())
    }

    /// Obtain the [`PublicKey`] part of this key pair, which can be used in calls to [`verify()`].
    /// or otherwise converted into an address. The public key bytes are equivalent to a Substrate
    /// `AccountId32`.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.public.to_bytes())
    }

    /// Sign some message. These bytes can be used directly in a Substrate `MultiSignature::sr25519(..)`.
    pub fn sign(&self, message: &[u8]) -> Signature {
        let context = schnorrkel::signing_context(SIGNING_CTX);
        let signature = self.0.sign(context.bytes(message));
        Signature(signature.to_bytes())
    }
}

/// Verify that some signature for a message was created by the owner of the [`PublicKey`].
///
/// ```rust
/// use subxt_signer::{ bip39::Mnemonic, sr25519 };
///
/// let keypair = sr25519::dev::alice();
/// let message = b"Hello!";
///
/// let signature = keypair.sign(message);
/// let public_key = keypair.public_key();
/// assert!(sr25519::verify(&signature, message, &public_key));
/// ```
pub fn verify<M: AsRef<[u8]>>(sig: &Signature, message: M, pubkey: &PublicKey) -> bool {
    let Ok(signature) = schnorrkel::Signature::from_bytes(&sig.0) else {
        return false;
    };
    let Ok(public) = schnorrkel::PublicKey::from_bytes(&pubkey.0) else {
        return false;
    };
    public
        .verify_simple(SIGNING_CTX, message.as_ref(), &signature)
        .is_ok()
}

/// An error handed back if creating a keypair fails.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid seed.
    #[error("Invalid seed (was it the wrong length?)")]
    InvalidSeed,
    /// Invalid phrase.
    #[error("Cannot parse phrase: {0}")]
    Phrase(#[from] bip39::Error),
    /// Invalid hex.
    #[error("Cannot parse hex string: {0}")]
    Hex(#[from] hex::FromHexError),
}

/// Dev accounts, helpful for testing but not to be used in production,
/// since the secret keys are known.
pub mod dev {
    use super::*;
    use std::str::FromStr;

    once_static_cloned! {
        /// Equivalent to `{DEV_PHRASE}//Alice`.
        pub fn alice() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Alice").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Bob`.
        pub fn bob() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Bob").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Charlie`.
        pub fn charlie() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Charlie").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Dave`.
        pub fn dave() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Dave").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Eve`.
        pub fn eve() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Eve").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Ferdie`.
        pub fn ferdie() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Ferdie").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//One`.
        pub fn one() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//One").unwrap()).unwrap()
        }
        /// Equivalent to `{DEV_PHRASE}//Two`.
        pub fn two() -> Keypair {
            Keypair::from_uri(&SecretUri::from_str("//Two").unwrap()).unwrap()
        }
    }
}

// Make `Keypair` usable to sign transactions in Subxt. This is optional so that
// `subxt-signer` can be used entirely independently of Subxt.
#[cfg(feature = "subxt")]
mod subxt_compat {
    use super::*;

    use subxt::config::Config;
    use subxt::tx::Signer as SignerT;
    use subxt::utils::{AccountId32, MultiAddress, MultiSignature};

    impl From<Signature> for MultiSignature {
        fn from(value: Signature) -> Self {
            MultiSignature::Sr25519(value.0)
        }
    }
    impl From<PublicKey> for AccountId32 {
        fn from(value: PublicKey) -> Self {
            value.to_account_id()
        }
    }
    impl<T> From<PublicKey> for MultiAddress<AccountId32, T> {
        fn from(value: PublicKey) -> Self {
            value.to_address()
        }
    }

    impl PublicKey {
        /// A shortcut to obtain an [`AccountId32`] from a [`PublicKey`].
        /// We often want this type, and using this method avoids any
        /// ambiguous type resolution issues.
        pub fn to_account_id(self) -> AccountId32 {
            AccountId32(self.0)
        }
        /// A shortcut to obtain a [`MultiAddress`] from a [`PublicKey`].
        /// We often want this type, and using this method avoids any
        /// ambiguous type resolution issues.
        pub fn to_address<T>(self) -> MultiAddress<AccountId32, T> {
            MultiAddress::Id(self.to_account_id())
        }
    }

    impl<T: Config> SignerT<T> for Keypair
    where
        T::AccountId: From<PublicKey>,
        T::Address: From<PublicKey>,
        T::Signature: From<Signature>,
    {
        fn account_id(&self) -> T::AccountId {
            self.public_key().into()
        }

        fn address(&self) -> T::Address {
            self.public_key().into()
        }

        fn sign(&self, signer_payload: &[u8]) -> T::Signature {
            self.sign(signer_payload).into()
        }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;

    use sp_core::crypto::Pair as _;
    use sp_core::sr25519::Pair as SpPair;

    #[test]
    fn check_from_phrase_matches() {
        for _ in 0..20 {
            let (sp_pair, phrase, _seed) = SpPair::generate_with_phrase(None);
            let phrase = bip39::Mnemonic::parse(phrase).expect("valid phrase expected");
            let pair = Keypair::from_phrase(&phrase, None).expect("should be valid");

            assert_eq!(sp_pair.public().0, pair.public_key().0);
        }
    }

    #[test]
    fn check_from_phrase_with_password_matches() {
        for _ in 0..20 {
            let (sp_pair, phrase, _seed) = SpPair::generate_with_phrase(Some("Testing"));
            let phrase = bip39::Mnemonic::parse(phrase).expect("valid phrase expected");
            let pair = Keypair::from_phrase(&phrase, Some("Testing")).expect("should be valid");

            assert_eq!(sp_pair.public().0, pair.public_key().0);
        }
    }

    #[test]
    fn check_from_secret_uri_matches() {
        // Some derive junctions to check that the logic there aligns:
        let uri_paths = [
            "/foo",
            "//bar",
            "/1",
            "/0001",
            "//1",
            "//0001",
            "//foo//bar/wibble",
            "//foo//001/wibble",
        ];

        for i in 0..2 {
            for path in &uri_paths {
                // Build an sp_core::Pair that includes a phrase, path and password:
                let password = format!("Testing{i}");
                let (_sp_pair, phrase, _seed) = SpPair::generate_with_phrase(Some(&password));
                let uri = format!("{phrase}{path}///{password}");
                let sp_pair = SpPair::from_string(&uri, None).expect("should be valid");

                // Now build a local Keypair using the equivalent API:
                let uri = SecretUri::from_str(&uri).expect("should be valid secret URI");
                let pair = Keypair::from_uri(&uri).expect("should be valid");

                // They should match:
                assert_eq!(sp_pair.public().0, pair.public_key().0);
            }
        }
    }

    #[test]
    fn check_dev_accounts_match() {
        use sp_keyring::sr25519::Keyring::*;

        assert_eq!(dev::alice().public_key().0, Alice.public().0);
        assert_eq!(dev::bob().public_key().0, Bob.public().0);
        assert_eq!(dev::charlie().public_key().0, Charlie.public().0);
        assert_eq!(dev::dave().public_key().0, Dave.public().0);
        assert_eq!(dev::eve().public_key().0, Eve.public().0);
        assert_eq!(dev::ferdie().public_key().0, Ferdie.public().0);
        assert_eq!(dev::one().public_key().0, One.public().0);
        assert_eq!(dev::two().public_key().0, Two.public().0);
    }

    #[test]
    fn check_signing_and_verifying_matches() {
        use sp_core::sr25519::Signature as SpSignature;

        for _ in 0..20 {
            let (sp_pair, phrase, _seed) = SpPair::generate_with_phrase(Some("Testing"));
            let phrase = bip39::Mnemonic::parse(phrase).expect("valid phrase expected");
            let pair = Keypair::from_phrase(&phrase, Some("Testing")).expect("should be valid");

            let message = b"Hello world";
            let sp_sig = sp_pair.sign(message).0;
            let sig = pair.sign(message).0;

            assert!(SpPair::verify(
                &SpSignature(sig),
                message,
                &sp_pair.public()
            ));
            assert!(verify(&Signature(sp_sig), message, &pair.public_key()));
        }
    }

    #[test]
    fn check_hex_uris() {
        // Hex URIs seem to ignore the password on sp_core and here. Check that this is consistent.
        let uri_str =
            "0x1122334455667788112233445566778811223344556677881122334455667788///SomePassword";

        let uri = SecretUri::from_str(uri_str).expect("should be valid");
        let pair = Keypair::from_uri(&uri).expect("should be valid");
        let sp_pair = SpPair::from_string(uri_str, None).expect("should be valid");

        assert_eq!(pair.public_key().0, sp_pair.public().0);
    }
}
