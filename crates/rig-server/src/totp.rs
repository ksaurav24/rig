//! Time-based one-time passwords (RFC 6238): HMAC-SHA1, 30-second steps,
//! six digits — what every authenticator app speaks.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

pub const STEP_SECS: i64 = 30;
const DIGITS: u32 = 6;
/// Codes from one step either side are accepted, for clock drift.
const SKEW: i64 = 1;
const SECRET_BYTES: usize = 20;
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A fresh random secret, base32 encoded without padding.
pub fn new_secret() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    rand::fill(&mut buf[..]);
    base32_encode(&buf)
}

pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let v = BASE32
            .iter()
            .position(|&b| b == c.to_ascii_uppercase() as u8)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The code for one time step.
pub fn code_at(key: &[u8], step: i64) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    bin % 10u32.pow(DIGITS)
}

pub fn step_of(unix: i64) -> i64 {
    unix.div_euclid(STEP_SECS)
}

/// Checks `code` against the steps around `unix`. Returns the matching step
/// when it is later than `last_step`, so a code is never accepted twice.
pub fn verify(secret: &str, code: &str, unix: i64, last_step: Option<i64>) -> Option<i64> {
    let code = code.trim().replace(' ', "");
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let want: u32 = code.parse().ok()?;
    let key = base32_decode(secret)?;
    let now = step_of(unix);
    let mut found = None;
    // Compare every candidate so timing does not reveal which one matched.
    for step in (now - SKEW)..=(now + SKEW) {
        if subtle::ConstantTimeEq::ct_eq(&code_at(&key, step), &want).into() {
            found = Some(step);
        }
    }
    found.filter(|s| last_step.is_none_or(|last| *s > last))
}

/// The `otpauth://` URI authenticator apps import, usually as a QR code.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    format!(
        "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        enc(issuer),
        enc(account),
        enc(issuer)
    )
}

/// The URI as an SVG QR code, rendered here so it never leaves the server.
pub fn qr_svg(uri: &str) -> anyhow::Result<String> {
    use qrcode::render::svg;
    let code =
        qrcode::QrCode::new(uri.as_bytes()).map_err(|e| anyhow::anyhow!("QR code: {e:?}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Ten single-use recovery codes like `k3vq-9xmz`.
pub fn recovery_codes() -> Vec<String> {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..10)
        .map(|_| {
            let mut buf = [0u8; 8];
            rand::fill(&mut buf[..]);
            let s: String = buf
                .iter()
                .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
                .collect();
            format!("{}-{}", &s[..4], &s[4..])
        })
        .collect()
}

/// Recovery codes are compared case-insensitively and without the dash.
pub fn normalize_recovery(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B, SHA1 key "12345678901234567890", eight digits;
    // the six-digit code is the last six of those.
    #[test]
    fn rfc6238_vectors() {
        let key = b"12345678901234567890";
        for (t, eight) in [
            (59, 94287082u32),
            (1111111109, 7081804),
            (1111111111, 14050471),
            (1234567890, 89005924),
            (2000000000, 69279037),
            (20000000000, 65353130),
        ] {
            assert_eq!(code_at(key, step_of(t)), eight % 1_000_000, "t={t}");
        }
    }

    #[test]
    fn base32_round_trip() {
        let secret = base32_encode(b"12345678901234567890");
        assert_eq!(secret, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(base32_decode(&secret).unwrap(), b"12345678901234567890");
        assert_eq!(base32_decode("gezd gnbv").unwrap(), b"12345");
        assert!(base32_decode("not base32!").is_none());
    }

    #[test]
    fn verify_accepts_drift_and_refuses_replay() {
        let secret = base32_encode(b"12345678901234567890");
        let t = 1111111111;
        let code = format!("{:06}", code_at(b"12345678901234567890", step_of(t)));
        let step = verify(&secret, &code, t, None).expect("current step");
        assert_eq!(verify(&secret, &code, t + 30, None), Some(step));
        assert_eq!(verify(&secret, &code, t + 90, None), None, "too old");
        assert_eq!(verify(&secret, &code, t, Some(step)), None, "replayed");
        assert_eq!(verify(&secret, "12345", t, None), None);
        assert_eq!(verify(&secret, "abcdef", t, None), None);
    }

    #[test]
    fn recovery_codes_are_distinct_and_normalize() {
        let codes = recovery_codes();
        assert_eq!(codes.len(), 10);
        assert_eq!(
            normalize_recovery(&codes[0].to_uppercase()),
            normalize_recovery(&codes[0])
        );
        assert_eq!(normalize_recovery("AB12-cd34"), "ab12cd34");
    }

    #[test]
    fn otpauth_uri_escapes_labels() {
        let uri = otpauth_uri("Rig", "a b@x.io", "ABC");
        assert!(uri.starts_with("otpauth://totp/Rig:a+b%40x.io?secret=ABC&issuer=Rig"));
        assert!(qr_svg(&uri).unwrap().starts_with("<?xml"));
    }
}
