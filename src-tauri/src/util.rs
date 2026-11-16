//! Small shared helpers.

use std::path::Path;

/// RFC3339 timestamp in UTC. Every persisted time in this app is one of these strings:
/// they sort lexicographically, survive SQLite round-trips, and read fine in a log.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A filesystem-safe fragment for building directory names out of user-supplied text.
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// How much of `bytes` can be decoded now: everything but a character cut off at the end.
///
/// Output arrives in pieces cut wherever a read happened to stop, which is often through
/// the middle of a multi-byte character. Decoding each piece on its own turns both halves
/// of that character into U+FFFD. Bytes that can never become valid are not held back -
/// they are garbage, and waiting for more would only delay them.
pub fn utf8_complete_len(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => {
            // Invalid somewhere in the middle; only the very end might be a cut character.
            // A UTF-8 sequence is at most four bytes, so look no further back than three.
            let start = bytes.len().saturating_sub(3);
            for at in (start..bytes.len()).rev() {
                let byte = bytes[at];
                if byte & 0b1100_0000 != 0b1000_0000 {
                    // A lead byte: is the sequence it starts cut short?
                    let wanted = match byte {
                        0xC2..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF4 => 4,
                        _ => return bytes.len(),
                    };
                    let have = bytes.len() - at;
                    return if have < wanted
                        && std::str::from_utf8(&bytes[at..])
                            .err()
                            .is_some_and(|e| e.error_len().is_none())
                    {
                        at
                    } else {
                        bytes.len()
                    };
                }
            }
            bytes.len()
        }
    }
}

pub fn dir_size_bytes(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_is_safe_and_bounded() {
        assert_eq!(slugify("Boltz-2 run #3"), "boltz-2-run-3");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("!!!"), "untitled");
        assert_eq!(slugify(&"a".repeat(200)).len(), 48);
        // Text in any other script is not a name a file system will mind, but it is not
        // ASCII either, and an empty slug would make every such job share a directory.
        assert_eq!(slugify("단백질 구조"), "untitled");
        assert_eq!(slugify("a/../b\n$(x)"), "a-b-x");
    }

    #[test]
    fn a_character_cut_in_half_waits_for_its_other_half() {
        let text = "가나".as_bytes(); // two characters of three bytes each
        assert_eq!(utf8_complete_len(text), 6);
        assert_eq!(utf8_complete_len(&text[..4]), 3);
        assert_eq!(utf8_complete_len(&text[..5]), 3);
        assert_eq!(utf8_complete_len(b""), 0);
        assert_eq!(utf8_complete_len(b"plain"), 5);
        // Garbage that can never be valid is passed on rather than held for ever...
        assert_eq!(utf8_complete_len(b"a\xffb"), 3);
        assert_eq!(utf8_complete_len(b"\xff"), 1);
        // ...but a cut character after it is still held back.
        let mut mixed = b"a\xffb".to_vec();
        mixed.extend_from_slice(&"가".as_bytes()[..2]);
        assert_eq!(utf8_complete_len(&mixed), 3);
        // A four-byte character cut after its first byte.
        let emoji = "🧬".as_bytes();
        assert_eq!(utf8_complete_len(&emoji[..1]), 0);
        assert_eq!(utf8_complete_len(&emoji[..3]), 0);
        assert_eq!(utf8_complete_len(emoji), 4);
    }
}
