#[inline(always)]
pub fn extract_u64_after_prefix(logs: &[String], prefix: &str) -> Option<u64> {
    logs.iter()
        .find_map(|log| extract_u64_from_line(log, prefix))
}

#[inline(always)]
pub fn extract_i64_after_prefix(logs: &[String], prefix: &str) -> Option<i64> {
    logs.iter()
        .find_map(|log| extract_i64_from_line(log, prefix))
}

#[inline(always)]
fn extract_u64_from_line(log: &str, prefix: &str) -> Option<u64> {
    let prefix_start = log.find(prefix)?;
    let value_start = prefix_start.checked_add(prefix.len())?;
    let value_slice = log.get(value_start..)?;
    parse_ascii_u64_prefix(value_slice)
}

#[inline(always)]
fn extract_i64_from_line(log: &str, prefix: &str) -> Option<i64> {
    let prefix_start = log.find(prefix)?;
    let value_start = prefix_start.checked_add(prefix.len())?;
    let value_slice = log.get(value_start..)?;

    if let Some(unsigned_slice) = value_slice.strip_prefix('-') {
        let unsigned_value = parse_ascii_u64_prefix(unsigned_slice)?;
        let signed_value = i64::try_from(unsigned_value).ok()?;
        return signed_value.checked_neg();
    }

    let unsigned_value = parse_ascii_u64_prefix(value_slice)?;
    i64::try_from(unsigned_value).ok()
}

#[inline(always)]
fn parse_ascii_u64_prefix(value: &str) -> Option<u64> {
    let mut result = 0_u64;
    let mut parsed_digit = false;

    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            break;
        }

        parsed_digit = true;
        result = result
            .checked_mul(10)?
            .checked_add(u64::from(byte.saturating_sub(b'0')))?;
    }

    if parsed_digit { Some(result) } else { None }
}

#[cfg(test)]
mod tests {
    use super::{extract_i64_after_prefix, extract_u64_after_prefix};

    #[test]
    fn extracts_unsigned_value() {
        let logs = vec![
            "Program log: unrelated".to_owned(),
            "Program log: vault_0_amount:12345, vault_1_amount:67890".to_owned(),
        ];

        assert_eq!(
            extract_u64_after_prefix(&logs, "vault_0_amount:"),
            Some(12_345)
        );
        assert_eq!(
            extract_u64_after_prefix(&logs, "vault_1_amount:"),
            Some(67_890)
        );
    }

    #[test]
    fn extracts_signed_value() {
        let logs = vec!["Program log: open_time: -42, slot: 7".to_owned()];

        assert_eq!(extract_i64_after_prefix(&logs, "open_time: "), Some(-42));
    }

    #[test]
    fn returns_none_when_prefix_or_digits_missing() {
        let logs = vec!["Program log: open_time: n/a".to_owned()];

        assert_eq!(extract_u64_after_prefix(&logs, "open_time: "), None);
        assert_eq!(extract_i64_after_prefix(&logs, "missing: "), None);
    }
}
