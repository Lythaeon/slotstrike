const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Lamports(u64);

impl Lamports {
    #[inline(always)]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub fn as_sol_string(self) -> String {
        let whole = self.0 / LAMPORTS_PER_SOL;
        let fractional = self.0 % LAMPORTS_PER_SOL;

        if fractional == 0 {
            return whole.to_string();
        }

        let mut fractional_string = format!("{fractional:09}");
        while fractional_string.ends_with('0') {
            fractional_string.pop();
        }

        format!("{whole}.{fractional_string}")
    }
}

pub fn parse_positive_sol_str_to_lamports(sol: &str) -> Option<Lamports> {
    let lamports = parse_sol_str_to_lamports(sol.trim())?;
    if lamports == 0 {
        return None;
    }

    Some(Lamports::new(lamports))
}

fn parse_sol_str_to_lamports(sol: &str) -> Option<u64> {
    if sol.is_empty() || sol.starts_with('-') {
        return None;
    }

    let (whole_raw, fractional_raw) = match sol.split_once('.') {
        Some((whole, fractional)) if !fractional.contains('.') => (whole, fractional),
        Some(_) => return None,
        None => (sol, ""),
    };

    if whole_raw.is_empty() && fractional_raw.is_empty() {
        return None;
    }

    if !whole_raw.is_empty()
        && !whole_raw
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return None;
    }

    if !fractional_raw.is_empty()
        && !fractional_raw
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return None;
    }

    if fractional_raw.len() > 9 {
        return None;
    }

    let whole = if whole_raw.is_empty() {
        0_u64
    } else {
        parse_ascii_u64(whole_raw)?
    };

    let mut fractional = if fractional_raw.is_empty() {
        0_u64
    } else {
        parse_ascii_u64(fractional_raw)?
    };

    for _ in fractional_raw.len()..9 {
        fractional = fractional.checked_mul(10)?;
    }

    whole
        .checked_mul(LAMPORTS_PER_SOL)
        .and_then(|whole_lamports| whole_lamports.checked_add(fractional))
}

fn parse_ascii_u64(value: &str) -> Option<u64> {
    if value.is_empty() {
        return None;
    }

    let mut parsed = 0_u64;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }

        parsed = parsed
            .checked_mul(10)?
            .checked_add(u64::from(byte.saturating_sub(b'0')))?;
    }

    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::{Lamports, parse_positive_sol_str_to_lamports};

    #[test]
    fn parses_fixed_point_sol_amounts() {
        assert_eq!(
            parse_positive_sol_str_to_lamports("1").map(Lamports::as_u64),
            Some(1_000_000_000)
        );
        assert_eq!(
            parse_positive_sol_str_to_lamports("1.23").map(Lamports::as_u64),
            Some(1_230_000_000)
        );
        assert_eq!(
            parse_positive_sol_str_to_lamports(".5").map(Lamports::as_u64),
            Some(500_000_000)
        );
        assert_eq!(
            parse_positive_sol_str_to_lamports("0.000000001").map(Lamports::as_u64),
            Some(1)
        );
    }

    #[test]
    fn rejects_invalid_or_non_positive_values() {
        assert_eq!(parse_positive_sol_str_to_lamports("0"), None);
        assert_eq!(parse_positive_sol_str_to_lamports("0.000000000"), None);
        assert_eq!(parse_positive_sol_str_to_lamports("1.0000000001"), None);
        assert_eq!(parse_positive_sol_str_to_lamports("abc"), None);
        assert_eq!(parse_positive_sol_str_to_lamports("-1"), None);
    }

    #[test]
    fn formats_sol_strings_without_float_drift() {
        assert_eq!(Lamports::new(1_000_000_000).as_sol_string(), "1");
        assert_eq!(Lamports::new(1_230_000_000).as_sol_string(), "1.23");
        assert_eq!(Lamports::new(1).as_sol_string(), "0.000000001");
    }
}
