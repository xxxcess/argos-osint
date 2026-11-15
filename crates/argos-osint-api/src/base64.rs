//! Dependency-free bounded base64 codec for binary IPC payloads.

use anyhow::ensure;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode raw bytes as standard padded base64.
pub(crate) fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let packed = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(ALPHABET[((packed >> 18) & 0x3F) as usize]));
        out.push(char::from(ALPHABET[((packed >> 12) & 0x3F) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[((packed >> 6) & 0x3F) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[(packed & 0x3F) as usize])
        } else {
            '='
        });
    }
    out
}

/// Decode standard padded base64 while enforcing `max_decoded_bytes` before allocation.
pub(crate) fn decode(input: &str, max_decoded_bytes: usize) -> anyhow::Result<Vec<u8>> {
    let bytes = input.as_bytes();
    ensure!(bytes.len().is_multiple_of(4), "invalid base64 length");
    let max_encoded = max_decoded_bytes.div_ceil(3).saturating_mul(4);
    ensure!(bytes.len() <= max_encoded, "base64 payload exceeds limit");
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let is_last = (index + 1) * 4 == bytes.len();
        let pad = chunk.iter().rev().take_while(|&&byte| byte == b'=').count();
        ensure!(pad <= 2 && (pad == 0 || is_last), "invalid base64 padding");
        let v0 = decode_sextet(chunk[0])?;
        let v1 = decode_sextet(chunk[1])?;
        let v2 = if chunk[2] == b'=' {
            ensure!(chunk[3] == b'=', "invalid base64 padding");
            0
        } else {
            decode_sextet(chunk[2])?
        };
        let v3 = if chunk[3] == b'=' {
            0
        } else {
            decode_sextet(chunk[3])?
        };
        if pad == 2 {
            ensure!(v1 & 0x0F == 0, "non-canonical base64 padding");
        } else if pad == 1 {
            ensure!(v2 & 0x03 == 0, "non-canonical base64 padding");
        }
        let packed = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
        out.push(((packed >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((packed >> 8) & 0xFF) as u8);
        }
        if pad == 0 {
            out.push((packed & 0xFF) as u8);
        }
    }
    ensure!(
        out.len() <= max_decoded_bytes,
        "decoded base64 payload exceeds limit"
    );
    Ok(out)
}

fn decode_sextet(byte: u8) -> anyhow::Result<u32> {
    match byte {
        b'A'..=b'Z' => Ok(u32::from(byte - b'A')),
        b'a'..=b'z' => Ok(u32::from(byte - b'a' + 26)),
        b'0'..=b'9' => Ok(u32::from(byte - b'0' + 52)),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => anyhow::bail!("invalid base64 character"),
    }
}
