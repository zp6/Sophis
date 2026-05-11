//! BIP-39 mnemonic generation, validation, and PBKDF2 seed derivation
//! for Sophis, plus the BIP-32 metadata types (`DerivationPath`,
//! `KeyFingerprint`, `ChildNumber`, `ExtendedKeyAttrs`,
//! `AddressType`) that downstream crates use for key-path bookkeeping.

mod address_type;
mod attrs;
mod child_number;
mod derivation_path;
mod error;
mod mnemonic;
pub mod types;

pub mod wasm {
    //! WASM bindings for the `bip32` module.
    pub use crate::mnemonic::{Language, Mnemonic, WordCount};
}

pub use address_type::AddressType;
pub use attrs::ExtendedKeyAttrs;
pub use child_number::ChildNumber;
pub use derivation_path::DerivationPath;
pub use mnemonic::{Language, Mnemonic, WordCount};
pub use types::*;
