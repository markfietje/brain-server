//! The deterministic PII masking primitives, shared by the read gate
//! (`gate::redact_content`), the write screen (`gate::screen_source_prompt`),
//! and the Beacon public-KB seam (`kb::sanitize_public`) — ONE definition of
//! email/phone/card masking for every path that must never emit raw PII.

/// Mask every email-shaped run with the `[redacted:email]` placeholder.
pub fn mask_email(out: &mut String) {
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(idx) = rest.find('@') {
        // Walk back over the local part (contiguous email chars).
        let head = &rest[..idx];
        let local = head
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')))
            .next()
            .unwrap_or("");
        let local_len = local.len();
        let prefix = &head[..head.len() - local_len];
        result.push_str(prefix);
        result.push_str("[redacted:email]");
        rest = &rest[idx..];
        // Skip to the end of the domain (next whitespace or end).
        if let Some(sp) = rest.find(char::is_whitespace) {
            rest = &rest[sp..];
        } else {
            rest = "";
        }
    }
    result.push_str(rest);
    *out = result;
}

/// Mask runs of 10-15 digits (optionally separated by ` -().+`) with the
/// placeholder, so phone numbers (and card-number runs that share the shape
/// but aren't Luhn) never leak to non-admin readers.
pub fn mask_phone(out: &mut String) {
    let mut result = String::with_capacity(out.len());
    let bytes = out.as_bytes();
    let mut i = 0;
    let mut in_run = false;
    let mut run_start = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() || matches!(b, b' ' | b'-' | b'(' | b')' | b'+' | b'.') {
            if !in_run {
                in_run = true;
                run_start = i;
            }
            i += 1;
        } else {
            if in_run {
                let run = &out[run_start..i];
                if (10..=15).contains(&count_digits(run)) {
                    result.push_str("[redacted:phone]");
                } else {
                    result.push_str(run);
                }
                in_run = false;
            }
            // `i` is always on a char boundary here (runs are ASCII and we
            // consume full chars below), so slice the whole char — a byte-wise
            // `out[i..i+1]` panics on multi-byte input (e.g. '—').
            let ch = out[i..]
                .chars()
                .next()
                .expect("i < out.len() on run boundary");
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    if in_run {
        let run = &out[run_start..];
        if (10..=15).contains(&count_digits(run)) {
            result.push_str("[redacted:phone]");
        } else {
            result.push_str(run);
        }
    }
    *out = result;
}

fn count_digits(s: &str) -> usize {
    s.bytes().filter(|b| b.is_ascii_digit()).count()
}

/// Luhn checksum (ISO/IEC 7812). Standard double-every-second-digit-from-right
/// with the doubled>9 → -9 adjustment.
pub fn luhn_ok(digits: &[u8]) -> bool {
    if digits.len() < 2 {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for &d in digits.iter().rev() {
        if !d.is_ascii_digit() {
            return false;
        }
        let mut v = (d - b'0') as u32;
        if double {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
        double = !double;
    }
    sum.is_multiple_of(10)
}

/// Mask Luhn-valid 13–19 digit runs (Visa/Mastercard/Amex/Discover).
pub fn mask_card(out: &mut String) {
    let bytes = out.as_bytes();
    let mut i = 0;
    let mut ranges_to_mask: Vec<(usize, usize)> = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut digit_run: Vec<u8> = Vec::new();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                digit_run.push(bytes[i]);
                i += 1;
            }
            // Only runs in card range (13–19) that pass Luhn are cards.
            if (13..=19).contains(&digit_run.len()) && luhn_ok(&digit_run) {
                ranges_to_mask.push((start, i));
            }
        } else {
            i += 1;
        }
    }
    if ranges_to_mask.is_empty() {
        return;
    }
    // Rebuild left-to-right, splicing the placeholder in for each card run.
    let mut final_out = String::with_capacity(out.len());
    let mut cursor = 0usize;
    for (start, end) in &ranges_to_mask {
        final_out.push_str(&out[cursor..*start]);
        final_out.push_str("[redacted:card]");
        cursor = *end;
    }
    final_out.push_str(&out[cursor..]);
    *out = final_out;
}

/// Unconditional masking: all three passes, no principal argument. This is
/// the public-artifact posture — there is no reader who may see raw PII.
pub fn redact_unconditional(s: &str) -> String {
    // Order matters: mask_email first so phone/card masking doesn't mangle
    // the domain we just consumed; mask_phone before mask_card because the
    // 10–15 range never overlaps a real 16–19 card, so the two passes are
    // independent.
    let mut out = s.to_string();
    mask_email(&mut out);
    mask_phone(&mut out);
    mask_card(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_unconditional_masks_all_classes() {
        let out = redact_unconditional("mail a@b.com call 5551234567 pay 4111111111111111");
        assert!(out.contains("[redacted:email]"));
        assert!(out.contains("[redacted:phone]"));
        assert!(out.contains("[redacted:card]"));
        assert!(!out.contains("a@b.com"));
    }

    #[test]
    fn luhn_rejects_bad_checksum() {
        assert!(luhn_ok(b"4111111111111111"));
        assert!(!luhn_ok(b"4111111111111112"));
    }
}
