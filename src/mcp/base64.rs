const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Fast, dependency-free standard base64 encoding (RFC 4648) with branchless chunked processing.
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    let chunks = input.chunks_exact(3);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let (b0, b1, b2) = (chunk[0], chunk[1], chunk[2]);
        out.push(BASE64_CHARS[(b0 >> 2) as usize]);
        out.push(BASE64_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
        out.push(BASE64_CHARS[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize]);
        out.push(BASE64_CHARS[(b2 & 0x3F) as usize]);
    }

    match remainder.len() {
        1 => {
            let b0 = remainder[0];
            out.push(BASE64_CHARS[(b0 >> 2) as usize]);
            out.push(BASE64_CHARS[((b0 & 0x03) << 4) as usize]);
            out.push(b'=');
            out.push(b'=');
        }
        2 => {
            let b0 = remainder[0];
            let b1 = remainder[1];
            out.push(BASE64_CHARS[(b0 >> 2) as usize]);
            out.push(BASE64_CHARS[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
            out.push(BASE64_CHARS[((b1 & 0x0F) << 2) as usize]);
            out.push(b'=');
        }
        _ => {}
    }

    // BASE64_CHARS and '=' are guaranteed ASCII, so `out` is valid UTF-8.
    String::from_utf8(out).unwrap_or_default()
}

/// Fast, dependency-free standard base64 decoding (RFC 4648).
pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;

    for &b in input.as_bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("Invalid base64 character: {}", b as char)),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode_rfc_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_base64_decode_rfc_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert!(base64_decode("!@#$%").is_err());
    }
}
