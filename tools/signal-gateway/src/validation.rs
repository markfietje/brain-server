//! Input validation helpers - enterprise security
#![allow(clippy::manual_strip)]

/// Validate recipient format (UUID, phone E.164, or ACI)
pub fn validate_recipient(recipient: &str) -> Result<String, &'static str> {
    let trimmed = recipient.trim();

    if trimmed.is_empty() {
        return Err("Recipient cannot be empty");
    }

    // UUID format: 8-4-4-4-12 (36 chars with dashes)
    let is_uuid = trimmed.len() == 36
        && trimmed.chars().filter(|c| *c == '-').count() == 4
        && trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == '-');

    // Phone: E.164 format (+ followed by 7-14 digits)
    let is_phone = trimmed.starts_with('+')
        && trimmed[1..].chars().all(|c| c.is_ascii_digit())
        && trimmed.len() >= 8
        && trimmed.len() <= 15;

    // ACI format (u: followed by UUID)
    let is_aci = if let Some(uuid_part) = trimmed.strip_prefix("u:") {
        uuid_part.len() == 36
            && uuid_part.chars().filter(|c| *c == '-').count() == 4
            && uuid_part.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    } else {
        false
    };

    if is_uuid || is_phone || is_aci {
        Ok(trimmed.to_string())
    } else {
        Err("Invalid recipient: must be UUID, phone (+1234567890), or ACI (u:uuid)")
    }
}

/// Validate message content
pub fn validate_message(message: &str) -> Result<String, &'static str> {
    let trimmed = message.trim();

    if trimmed.is_empty() {
        return Err("Message cannot be empty");
    }

    if trimmed.len() > 10000 {
        return Err("Message too long (max 10000 chars)");
    }

    Ok(trimmed.to_string())
}

/// Validate phone number format only
pub fn validate_phone(phone: &str) -> Result<String, &'static str> {
    let trimmed = phone.trim();

    if trimmed.starts_with('+') {
        let stripped = &trimmed[1..];
        if stripped.chars().all(|c| c.is_ascii_digit())
            && stripped.len() >= 7
            && stripped.len() <= 14
        {
            return Ok(trimmed.to_string());
        }
    }

    Err("Invalid phone: must be E.164 format (+1234567890)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_recipient_uuid() {
        assert!(validate_recipient("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_recipient("invalid").is_err());
    }

    #[test]
    fn test_validate_recipient_phone() {
        assert!(validate_recipient("+1234567890").is_ok());
        assert!(validate_recipient("+44").is_err());
    }

    #[test]
    fn test_validate_recipient_aci() {
        assert!(validate_recipient("u:550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn test_validate_message() {
        assert!(validate_message("Hello").is_ok());
        assert!(validate_message("").is_err());
        assert!(validate_message(&"x".repeat(10001)).is_err());
    }

    #[test]
    fn test_validate_phone() {
        assert!(validate_phone("+1234567890").is_ok());
        assert!(validate_phone("+44").is_err());
    }
}
