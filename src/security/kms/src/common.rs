// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::fmt;
use std::str;

use crate::crypto_provider::CryptoProvider;
use fidl_fuchsia_kms::{
    AsymmetricKeyAlgorithm, AsymmetricPrivateKeyRequest, Error, KeyOrigin, KeyProvider,
};
use fidl_fuchsia_mem::Buffer;
use fuchsia_zircon as zx;
use serde::{Deserialize, Serialize};
use tracing::error;

/// Different type of key request.
pub enum KeyRequestType<'a> {
    /// Request on an asymmetric private key.
    AsymmetricPrivateKeyRequest(AsymmetricPrivateKeyRequest),
    /// Request on a sealing key to seal data.
    SealingKeyRequest(DataRequest<'a>),
    /// Request on a sealing key to unseal data.
    UnsealingKeyRequest(DataRequest<'a>),
}

/// A structure to contain a data buffer as input and a mutable result reference as output.
pub struct DataRequest<'a> {
    pub data: Buffer,
    pub result: &'a mut Result<Buffer, Error>,
}

/// The key types.
#[derive(Serialize, Deserialize, PartialEq, Copy, Clone)]
pub enum KeyType {
    /// A type of key used to seal and unseal data. This type of key is generated by KMS and not the
    /// user.
    SealingKey,
    /// A type of key representing an asymmetric private key. It could be used to sign data.
    AsymmetricPrivateKey,
}

/// A key object in memory.
///
/// Each key stored in KMS should have a singleton of key object in memory when used. The user are
/// given a handler associated with this unique in memory key object. If a key object does not exist
/// KMS would read from storage and create the key object. After all the handles to this key object
/// is dropped, the key object would be removed from memory. This ensures that all the operations
/// are synchronized based on lock to use this Key object.
pub trait KmsKey: Send + fmt::Debug {
    /// Whether this key is already deleted. A key may be deleted while there are other channels
    /// associated with that key. In this case, any operation on the deleted key would return
    /// key_not_found.
    fn is_deleted(&self) -> bool;
    /// Delete the key. This would include telling the crypto provider to delete key (If provider
    /// keeps some resource for the key) and set the deleted to true.
    ///
    /// # Panics
    ///
    /// Panics if the key should never be deleted, for example, the sealing key should never be
    /// deleted.
    fn delete(&mut self) -> Result<(), Error>;
    /// Get the name for the current key.
    fn get_key_name(&self) -> &str;
    /// Handle a request from user. Note that a key should only handle the request for that key
    /// and this is enforced by key_manager. KmsSealingKey is never expected to return a FIDL error
    /// when handling request, for detail, please see KmsSealingKey documentation.
    ///
    /// # Panics
    ///
    /// Panics if request type is invalid.
    fn handle_request(&self, req: KeyRequestType<'_>) -> Result<(), fidl::Error>;
    /// The the type for the current key.
    fn get_key_type(&self) -> KeyType;
    /// Get the crypto provider for the key.
    fn get_key_provider(&self) -> KeyProvider;
    /// Get the key data.
    fn get_key_data(&self) -> Vec<u8>;
}

/// A list of all the supported asymmetric key algorithms.
#[cfg(test)]
pub const ASYMMETRIC_KEY_ALGORITHMS: &[AsymmetricKeyAlgorithm] = &[
    AsymmetricKeyAlgorithm::EcdsaSha256P256,
    AsymmetricKeyAlgorithm::EcdsaSha512P384,
    AsymmetricKeyAlgorithm::EcdsaSha512P521,
    AsymmetricKeyAlgorithm::RsaSsaPssSha2562048,
    AsymmetricKeyAlgorithm::RsaSsaPssSha2563072,
    AsymmetricKeyAlgorithm::RsaSsaPssSha5124096,
    AsymmetricKeyAlgorithm::RsaSsaPkcs1Sha2562048,
    AsymmetricKeyAlgorithm::RsaSsaPkcs1Sha2563072,
    AsymmetricKeyAlgorithm::RsaSsaPkcs1Sha5124096,
];

/// The key attributes structure to be stored as attribute file.
pub struct KeyAttributes<'a> {
    pub asymmetric_key_algorithm: Option<AsymmetricKeyAlgorithm>,
    pub key_type: KeyType,
    pub key_origin: KeyOrigin,
    pub provider: &'a dyn CryptoProvider,
    pub key_data: Vec<u8>,
}

/// Emit a error message and return an error.
///
/// Invoke the `error!` macro on all but the first argument. A call to
/// `debug_err!(err, ...)` is an expression whose value is the expression `err`.
#[macro_export]
macro_rules! debug_err {
    ($err:expr, $($arg:tt)*) => (
        // TODO(joshlf): Uncomment once attributes are allowed on expressions
        // #[cfg_attr(feature = "cargo-clippy", allow(block_in_if_condition_stmt))]
        {
            use ::tracing::error;
            error!($($arg)*);
            $err
        }
    )
}

/// Create a closure which emits a error message and turn original error to a new error.
///
/// Creates a closure that would return the first argument and print an error message with the
/// original error that is used to call the closure as the last argument to the error message.
macro_rules! debug_err_fn {
    ($return_err:expr, $($arg:tt)*) => (
        |err| {
            use ::tracing::error;
            error!($($arg)*, err);
            $return_err
        }
    )
}

/// Create a closure which emits a error message and returns an error.
///
/// Creates a closure that takes no argument and return the first argument and print an error
/// message.
macro_rules! debug_err_fn_no_argument {
    ($return_err:expr, $($arg:tt)*) => (
        || {
            use ::tracing::error;
            error!($($arg)*);
            $return_err
        }
    )
}

#[cfg(test)]
pub fn generate_random_data(size: u32) -> Vec<u8> {
    use rand::Rng;
    let mut random_data = Vec::new();
    let mut rng = rand::thread_rng();
    for _i in 0..size {
        let byte: u8 = rng.gen();
        random_data.push(byte);
    }
    random_data
}

/// Read data from a VMO buffer.
pub fn buffer_to_data(buffer: Buffer) -> Result<Vec<u8>, Error> {
    let buffer_size = buffer.size;
    let buffer_vmo = buffer.vmo;

    let mut input = vec![0; buffer_size as usize];
    buffer_vmo
        .read(&mut input, 0)
        .map_err(debug_err_fn!(Error::InternalError, "Failed to read data from vmo: {:?}."))?;
    Ok(input)
}

/// Write data into a VMO buffer.
pub fn data_to_buffer(data: &[u8]) -> Result<Buffer, Error> {
    let vmo = zx::Vmo::create(data.len() as u64)
        .map_err(debug_err_fn!(Error::InternalError, "Failed to create vmo: {:?}"))?;
    vmo.write(&data, 0)
        .map_err(debug_err_fn!(Error::InternalError, "Failed to write data to vmo: {:?}"))?;
    Ok(Buffer { vmo, size: data.len() as u64 })
}
