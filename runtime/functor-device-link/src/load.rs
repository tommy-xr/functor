//! The headset's load status, headset → desktop: what its status card
//! says (receiving a project, checking code, loading assets, a failure), so
//! the desktop can say the same where a headset session is started. A
//! message when the step changes and as its count moves; an empty one when
//! the card hides.
//!
//! The first line is the step's kind — `waiting`, `connected`, `receiving`,
//! `starting`, `assets`, `running`, `reloading`, `failed` — and the rest are
//! the card's lines as the headset words them (heading first). A desktop
//! shows a kind it does not know as nothing.

/// Headset → desktop: the kind and the card's lines, UTF-8, one per line.
pub const CHANNEL: u16 = 5;

/// Most bytes of a message; longer is cut.
pub const MAX_BYTES: usize = 1024;

/// The message for `lines`.
pub fn encode(lines: &[String]) -> Vec<u8> {
    let mut text = lines.join("\n");
    while text.len() > MAX_BYTES {
        text.pop();
    }
    text.into_bytes()
}

/// The lines of a message; empty when the card is hidden. Bytes that are
/// not text, or too many, are no lines.
pub fn decode(data: &[u8]) -> Vec<String> {
    match std::str::from_utf8(data) {
        Ok(text) if data.len() <= MAX_BYTES && !text.is_empty() => {
            text.lines().map(|line| line.chars().filter(|c| !c.is_control()).collect()).collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip_and_garbage_is_no_lines() {
        let lines = vec!["RECEIVING PROJECT".to_string(), "23.0 of 67.5 MB (34%), 4 of 9 files".to_string()];
        assert_eq!(decode(&encode(&lines)), lines);
        assert!(decode(&encode(&[])).is_empty());
        assert!(decode(&[0xff, 0xfe]).is_empty());
        assert!(decode(&vec![b'a'; MAX_BYTES + 1]).is_empty());
        assert_eq!(encode(&["x".repeat(MAX_BYTES * 2)]).len(), MAX_BYTES);
        assert_eq!(decode(b"A\x07B"), ["AB"]);
    }
}
