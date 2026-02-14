use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
    sync::Arc,
};

use crate::domain::value_objects::sol_amount::Lamports;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuleAddress(Arc<str>);

impl RuleAddress {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, &'static str> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err("rule address must not be empty");
        }

        Ok(Self(value))
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for RuleAddress {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for RuleAddress {
    #[inline(always)]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Display for RuleAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for RuleAddress {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        RuleAddress::new(Arc::<str>::from(value))
    }
}

impl TryFrom<&str> for RuleAddress {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        RuleAddress::new(Arc::<str>::from(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuleSlippageBps(u16);

impl RuleSlippageBps {
    pub const MAX_BPS: u16 = 10_000;

    pub fn from_pct_str(value: &str) -> Result<Self, &'static str> {
        let value = value.trim();
        if value.is_empty() {
            return Err("slippage must not be empty");
        }

        let (whole_raw, fractional_raw) = value.split_once('.').unwrap_or((value, "0"));
        if fractional_raw.len() > 4 {
            return Err("slippage supports up to 4 decimal places");
        }

        let whole_part = whole_raw
            .parse::<u64>()
            .map_err(|_parse_error| "invalid slippage value")?;

        if !fractional_raw
            .chars()
            .all(|character| character.is_ascii_digit())
        {
            return Err("invalid slippage value");
        }

        let mut fractional_scaled = fractional_raw.to_owned();
        while fractional_scaled.len() < 4 {
            fractional_scaled.push('0');
        }

        let fractional_part = fractional_scaled
            .parse::<u64>()
            .map_err(|_parse_error| "invalid slippage value")?;

        let pct_scaled_4 = whole_part
            .checked_mul(10_000)
            .and_then(|scaled_value| scaled_value.checked_add(fractional_part))
            .ok_or("slippage value overflow")?;

        let bps = pct_scaled_4
            .checked_div(100)
            .ok_or("invalid slippage value")?;

        if bps > u64::from(Self::MAX_BPS) {
            return Err("slippage must be between 0 and 100");
        }

        let bps = u16::try_from(bps).map_err(|_conversion_error| "slippage value overflow")?;
        Ok(Self(bps))
    }

    #[inline(always)]
    pub const fn as_bps(self) -> u16 {
        self.0
    }

    pub fn as_pct_string(self) -> String {
        let whole = self.0 / 100;
        let fractional = self.0 % 100;
        format!("{whole}.{fractional:02}")
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuleSolAmount(Lamports);

impl RuleSolAmount {
    #[inline(always)]
    pub const fn new(lamports: Lamports) -> Self {
        Self(lamports)
    }

    #[inline(always)]
    pub const fn as_lamports(self) -> Lamports {
        self.0
    }

    #[inline(always)]
    pub fn as_sol_string(self) -> String {
        self.0.as_sol_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{RuleAddress, RuleSlippageBps};

    #[test]
    fn creates_non_empty_rule_address() {
        let address = RuleAddress::try_from("So11111111111111111111111111111111111111112");
        assert!(address.is_ok());

        let empty = RuleAddress::try_from(" ");
        assert!(empty.is_err());
    }

    #[test]
    fn parses_slippage_from_percent_without_float_math() {
        let parsed = RuleSlippageBps::from_pct_str("1.25");
        assert!(parsed.is_ok());

        if let Ok(value) = parsed {
            assert_eq!(value.as_bps(), 125);
            assert_eq!(value.as_pct_string(), "1.25");
        }
    }

    #[test]
    fn rejects_slippage_outside_bounds() {
        assert!(RuleSlippageBps::from_pct_str("100.01").is_err());
        assert!(RuleSlippageBps::from_pct_str("abc").is_err());
    }
}
