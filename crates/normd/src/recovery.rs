//! Turning a vault key into something a person can write on paper.
//!
//! # Why not raw hex
//!
//! Sixty-four hex characters is correct and awful. Worse than awful: it has no
//! way to tell a good transcription from a bad one. Mistype one character
//! while setting up a second device and the key is simply wrong — the failure
//! surfaces later, as chunks that will not decrypt, and reads to the user like
//! their notes are gone rather than like they made a typo.
//!
//! So the phrase carries a checksum. A wrong phrase is rejected at the moment
//! it is typed, with a message that says what happened.
//!
//! # Crockford base32
//!
//! `I`, `L`, `O` and `U` are absent, which removes the confusions that matter
//! on paper: 1/I/l and 0/O. On the way back in, `I` and `L` are read as `1` and
//! `O` as `0`, case is ignored, and any grouping the user adds or drops is
//! ignored too. Someone copying the phrase by hand should not be able to get it
//! subtly wrong.
//!
//! # This is not BIP39
//!
//! Twenty-four words would be easier still to write down and to read back over
//! the phone, and it is where this should end up. It needs the standard's exact
//! 2048-word list — an approximation of it would produce phrases that look
//! interoperable and are not, which is worse than not claiming to be. Until
//! that list is vendored in and checked, the phrase is what it is here, and it
//! is not described as anything else.

use anyhow::{bail, Context, Result};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const GROUP: usize = 5;
const KEY_LEN: usize = 32;
const CHECK_LEN: usize = 2;

/// Renders a key as a grouped, checksummed phrase.
pub fn encode(key: &[u8; KEY_LEN]) -> String {
    let mut payload = key.to_vec();
    payload.extend_from_slice(&checksum(key));

    let encoded = base32_encode(&payload);
    encoded
        .as_bytes()
        .chunks(GROUP)
        .map(|c| std::str::from_utf8(c).expect("alphabet is ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Reads a phrase back, rejecting anything that does not check out.
pub fn decode(phrase: &str) -> Result<[u8; KEY_LEN]> {
    let cleaned: String = phrase
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| match c.to_ascii_uppercase() {
            // The substitutions Crockford specifies, so a phrase copied by
            // hand from paper still works.
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        })
        .collect();

    let bytes =
        base32_decode(&cleaned).context("recovery phrase contains an unusable character")?;

    if bytes.len() < KEY_LEN + CHECK_LEN {
        bail!(
            "recovery phrase is too short — expected {} characters, got {}",
            encode(&[0u8; KEY_LEN])
                .chars()
                .filter(|c| *c != '-')
                .count(),
            cleaned.len()
        );
    }

    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes[..KEY_LEN]);

    if bytes[KEY_LEN..KEY_LEN + CHECK_LEN] != checksum(&key) {
        bail!(
            "recovery phrase does not check out — it was probably mistyped. \
             Compare it against what `normd init` printed; nothing is lost by \
             trying again."
        );
    }

    Ok(key)
}

fn checksum(key: &[u8; KEY_LEN]) -> [u8; CHECK_LEN] {
    let full = blake3::hash(key);
    let mut out = [0u8; CHECK_LEN];
    out.copy_from_slice(&full.as_bytes()[..CHECK_LEN]);
    out
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits = 0;

    for b in bytes {
        acc = (acc << 8) | *b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;

    for c in s.bytes() {
        let Some(v) = ALPHABET.iter().position(|a| *a == c) else {
            bail!("'{}' is not part of the alphabet", c as char);
        };
        acc = (acc << 5) | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }

    // 34 bytes need 55 characters, which carry 275 bits — three more than the
    // payload uses. Left unchecked, those three bits are a hole in the
    // checksum: the final character can be mistyped into any of eight values
    // and decode to exactly the same key. Requiring them to be zero closes it.
    if bits > 0 && acc & ((1 << bits) - 1) != 0 {
        bail!("the last character of the recovery phrase is wrong");
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> [u8; KEY_LEN] {
        let mut k = [0u8; KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = n.wrapping_add(i as u8);
        }
        k
    }

    #[test]
    fn round_trips() {
        for n in [0, 1, 7, 200, 255] {
            assert_eq!(decode(&encode(&key(n))).unwrap(), key(n));
        }
    }

    #[test]
    fn is_grouped_and_uses_no_confusable_letters() {
        let phrase = encode(&key(3));
        assert!(phrase.contains('-'), "not grouped: {phrase}");
        for c in phrase.chars().filter(|c| *c != '-') {
            assert!(
                !"ILOU".contains(c),
                "{c} is easy to misread by hand, in {phrase}"
            );
        }
    }

    #[test]
    fn a_single_mistyped_character_is_caught() {
        // The whole point. Without this the mistake surfaces much later as
        // chunks that will not decrypt.
        let phrase = encode(&key(9));
        let mut wrong: Vec<char> = phrase.chars().collect();
        for i in 0..wrong.len() {
            if wrong[i] == '-' {
                continue;
            }
            let original = wrong[i];
            wrong[i] = if original == '7' { '8' } else { '7' };
            let candidate: String = wrong.iter().collect();
            assert!(
                decode(&candidate).is_err(),
                "a typo at position {i} was accepted: {candidate}"
            );
            wrong[i] = original;
        }
    }

    #[test]
    fn hand_copying_quirks_are_forgiven() {
        let phrase = encode(&key(11));
        let expected = decode(&phrase).unwrap();

        for variant in [
            phrase.to_lowercase(),
            phrase.replace('-', ""),
            phrase.replace('-', " "),
            format!("  {phrase}  "),
            phrase.replace('-', "\n"),
        ] {
            assert_eq!(decode(&variant).unwrap(), expected, "rejected: {variant:?}");
        }
    }

    #[test]
    fn confusable_letters_are_read_as_their_digits() {
        // Someone writing `0` and reading it back as `O`.
        let phrase = encode(&key(13));
        let as_written = phrase.replace('0', "O").replace('1', "I");
        assert_eq!(decode(&as_written).unwrap(), key(13));
    }

    #[test]
    fn a_truncated_phrase_is_refused() {
        let phrase = encode(&key(5));
        assert!(decode(&phrase[..phrase.len() / 2]).is_err());
        assert!(decode("").is_err());
    }

    #[test]
    fn two_keys_never_share_a_phrase() {
        assert_ne!(encode(&key(1)), encode(&key(2)));
    }
}
