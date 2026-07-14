//! Helpers for ANSSI R23 (secure erasure of sensitive buffers).

use zeroize::Zeroize;

pub fn zeroize_vec(buffer: &mut Vec<u8>) {
    buffer.zeroize();
}
