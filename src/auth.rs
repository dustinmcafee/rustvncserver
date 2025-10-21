// Copyright 2025 Dustin McAfee
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! VNC authentication implementation.
//!
//! This module implements VNC Authentication (security type 2) as specified in RFC 6143 Section 7.2.2.
//! It uses DES encryption with a VNC-specific bit reversal quirk for challenge-response authentication.
//!
//! # Protocol
//!
//! The VNC authentication handshake works as follows:
//! 1. Server generates a 16-byte random challenge
//! 2. Server sends the challenge to the client
//! 3. Client encrypts the challenge using the password as the DES key (with bit-reversed bytes)
//! 4. Client sends the encrypted result back to the server
//! 5. Server verifies the response matches its own encryption of the challenge
//!
//! # Security Note
//!
//! VNC Authentication is a legacy protocol and has known security limitations. It should only
//! be used on trusted networks or in conjunction with TLS/SSL tunneling.

use des::cipher::{BlockEncrypt, KeyInit};
use des::Des;
use rand::Rng;

/// Handles VNC authentication, specifically the VNC Authentication scheme as defined in RFC 6143 Section 7.2.2.
///
/// This struct is responsible for managing the VNC server's password, generating a secure challenge
/// for clients, and verifying their responses using DES encryption with a VNC-specific bit reversal quirk.
pub struct VncAuth {
    /// The VNC password, if set. Stored as an `Option<String>`.
    password: Option<String>,
}

impl VncAuth {
    /// Creates a new `VncAuth` instance.
    ///
    /// # Arguments
    ///
    /// * `password` - An `Option<String>` containing the VNC password. If `None`, no password is set.
    ///
    /// # Returns
    ///
    /// A new `VncAuth` object.
    pub fn new(password: Option<String>) -> Self {
        Self { password }
    }

    /// Generates a cryptographically random 16-byte challenge for VNC authentication.
    ///
    /// This challenge is sent to the client, which must encrypt it with the shared secret (password)
    /// and send the result back for verification.
    ///
    /// # Returns
    ///
    /// A `[u8; 16]` array containing the random challenge bytes.
    #[allow(clippy::unused_self)] // Kept as method for API consistency with other VncAuthenticator methods
    pub fn generate_challenge(&self) -> [u8; 16] {
        let mut rng = rand::thread_rng();
        let mut challenge = [0u8; 16];
        rng.fill(&mut challenge);
        challenge
    }

    /// Verifies a client's authentication response against the generated challenge and the server's password.
    ///
    /// The client's response is expected to be the challenge encrypted with the VNC password.
    /// This function re-encrypts the original challenge with the stored password and compares it
    /// to the client's provided `response`.
    ///
    /// # Arguments
    ///
    /// * `response` - A slice of bytes containing the client's encrypted response (16 bytes).
    /// * `challenge` - The original 16-byte challenge that was sent to the client.
    ///
    /// # Returns
    ///
    /// `true` if the response matches the expected encrypted challenge, `false` otherwise.
    pub fn verify_response(&self, response: &[u8], challenge: &[u8; 16]) -> bool {
        if let Some(ref password) = self.password {
            let expected = self.encrypt_challenge(challenge, password);
            response == expected.as_slice()
        } else {
            false
        }
    }

    /// Encrypts a 16-byte challenge with the VNC password using DES encryption.
    ///
    /// This function implements the VNC-specific DES encryption, which involves
    /// reversing the bits of each password byte before using it as the DES key.
    /// The 16-byte challenge is encrypted as two 8-byte DES blocks in ECB mode.
    ///
    /// # Arguments
    ///
    /// * `challenge` - A 16-byte array representing the challenge to be encrypted.
    /// * `password` - The VNC password string.
    ///
    /// # Returns
    ///
    /// A `Vec<u8>` containing the 16-byte encrypted challenge.
    #[allow(clippy::unused_self)] // Kept as method for API consistency with other VncAuthenticator methods
    fn encrypt_challenge(&self, challenge: &[u8; 16], password: &str) -> Vec<u8> {
        // Prepare VNC password key (8 bytes, bit-reversed)
        let mut key = [0u8; 8];
        let pw_bytes = password.as_bytes();

        // Copy password bytes (up to 8), truncate or pad with zeros
        for (i, &byte) in pw_bytes.iter().take(8).enumerate() {
            key[i] = reverse_bits(byte);
        }

        // Create DES cipher with the VNC key
        let cipher = Des::new_from_slice(&key).expect("8-byte key");

        // Encrypt the 16-byte challenge as two 8-byte blocks (DES ECB mode)
        let mut encrypted = vec![0u8; 16];

        // First block
        let mut block1_bytes = [0u8; 8];
        block1_bytes.copy_from_slice(&challenge[0..8]);
        let mut block1 = block1_bytes.into();
        cipher.encrypt_block(&mut block1);
        encrypted[0..8].copy_from_slice(&block1);

        // Second block
        let mut block2_bytes = [0u8; 8];
        block2_bytes.copy_from_slice(&challenge[8..16]);
        let mut block2 = block2_bytes.into();
        cipher.encrypt_block(&mut block2);
        encrypted[8..16].copy_from_slice(&block2);

        encrypted
    }
}

/// Reverses the bits within a single byte.
///
/// This utility function is used specifically in VNC authentication to implement a historical
/// quirk where password bytes have their bits reversed before being used as a DES key.
///
/// # Arguments
///
/// * `byte` - The `u8` value whose bits are to be reversed.
///
/// # Returns
///
/// The `u8` value with its bits reversed.
///
/// # Example
///
/// `0b10110001` (177) becomes `0b10001101` (141).
fn reverse_bits(byte: u8) -> u8 {
    let mut result = 0u8;
    for i in 0..8 {
        if byte & (1 << i) != 0 {
            result |= 1 << (7 - i);
        }
    }
    result
}
