//! Connection encryption: AES-128 in CFB-8 mode (vanilla: `NettyEncryptingEncoder` /
//! `NettyEncryptingDecoder`).
//!
//! After the login key exchange both directions are encrypted with the 16-byte shared
//! secret used as key **and** initial vector, each direction keeping its own state for the
//! lifetime of the connection.

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};

/// One direction of a CFB-8 stream.
#[derive(Clone)]
pub struct Cfb8 {
    cipher: Aes128,
    register: [u8; 16],
    decrypt: bool,
}

impl std::fmt::Debug for Cfb8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cfb8")
            .field("decrypt", &self.decrypt)
            .finish_non_exhaustive()
    }
}

impl Cfb8 {
    fn new(secret: &[u8; 16], decrypt: bool) -> Self {
        Self {
            cipher: Aes128::new(GenericArray::from_slice(secret)),
            register: *secret,
            decrypt,
        }
    }

    /// An encrypting stream for data sent with `secret`.
    pub fn encryptor(secret: &[u8; 16]) -> Self {
        Self::new(secret, false)
    }

    /// A decrypting stream for data received with `secret`.
    pub fn decryptor(secret: &[u8; 16]) -> Self {
        Self::new(secret, true)
    }

    /// Encrypts or decrypts `data` in place, continuing the stream.
    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            let mut block = GenericArray::clone_from_slice(&self.register);
            self.cipher.encrypt_block(&mut block);
            let input = *byte;
            let output = input ^ block[0];
            *byte = output;
            let cipher_byte = if self.decrypt { input } else { output };
            self.register.copy_within(1.., 0);
            self.register[15] = cipher_byte;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// NIST SP 800-38A F.3.7 (CFB8-AES128.Encrypt), with its own IV.
    #[test]
    fn nist_vector() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let mut enc = Cfb8::encryptor(&key);
        enc.register = iv;
        let mut data = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        enc.apply(&mut data);
        assert_eq!(data, hex("3b79424c9c0dd436bace9e0ed4586a4f32b9"));
    }

    #[test]
    fn round_trip_in_pieces() {
        let secret = [7u8; 16];
        let mut enc = Cfb8::encryptor(&secret);
        let mut dec = Cfb8::decryptor(&secret);
        let original: Vec<u8> = (0..100u8).collect();
        let mut data = original.clone();
        enc.apply(&mut data[..33]);
        enc.apply(&mut data[33..]);
        assert_ne!(data, original);
        dec.apply(&mut data[..1]);
        dec.apply(&mut data[1..]);
        assert_eq!(data, original);
    }
}
