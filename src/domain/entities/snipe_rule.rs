use std::sync::Arc;

use crate::domain::value_objects::{RuleAddress, RuleSlippageBps, RuleSolAmount};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnipeRuleHot {
    snipe_height: RuleSolAmount,
    jito_tip: RuleSolAmount,
    slippage: RuleSlippageBps,
}

impl SnipeRuleHot {
    #[inline(always)]
    pub const fn new(
        snipe_height: RuleSolAmount,
        jito_tip: RuleSolAmount,
        slippage: RuleSlippageBps,
    ) -> Self {
        Self {
            snipe_height,
            jito_tip,
            slippage,
        }
    }

    #[inline(always)]
    pub const fn snipe_height(self) -> RuleSolAmount {
        self.snipe_height
    }

    #[inline(always)]
    pub const fn jito_tip(self) -> RuleSolAmount {
        self.jito_tip
    }

    #[inline(always)]
    pub const fn slippage(self) -> RuleSlippageBps {
        self.slippage
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnipeRuleCold {
    pub address: RuleAddress,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnipeRule {
    hot: SnipeRuleHot,
    cold: Arc<SnipeRuleCold>,
}

impl SnipeRule {
    #[inline(always)]
    pub fn new(
        address: RuleAddress,
        snipe_height: RuleSolAmount,
        jito_tip: RuleSolAmount,
        slippage: RuleSlippageBps,
    ) -> Self {
        Self {
            hot: SnipeRuleHot::new(snipe_height, jito_tip, slippage),
            cold: Arc::new(SnipeRuleCold { address }),
        }
    }

    #[inline(always)]
    pub const fn hot(&self) -> SnipeRuleHot {
        self.hot
    }

    #[inline(always)]
    pub fn cold_arc(&self) -> Arc<SnipeRuleCold> {
        Arc::clone(&self.cold)
    }

    #[inline(always)]
    pub fn address(&self) -> &RuleAddress {
        &self.cold.address
    }

    #[inline(always)]
    pub const fn snipe_height(&self) -> RuleSolAmount {
        self.hot.snipe_height
    }

    #[inline(always)]
    pub const fn jito_tip(&self) -> RuleSolAmount {
        self.hot.jito_tip
    }

    #[inline(always)]
    pub const fn slippage(&self) -> RuleSlippageBps {
        self.hot.slippage
    }

    pub fn as_log_line(&self, label: &str) -> String {
        format!(
            "{} > {} \\n\t\t\tSnipe height: {} SOL \\n\t\t\tJito tip: {} SOL \\n\t\t\tSlippage: {} %",
            label,
            self.address(),
            self.snipe_height().as_sol_string(),
            self.jito_tip().as_sol_string(),
            self.slippage().as_pct_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SnipeRule;
    use crate::domain::value_objects::{
        RuleAddress, RuleSlippageBps, RuleSolAmount, sol_amount::Lamports,
    };

    fn build_rule() -> Option<SnipeRule> {
        let address = RuleAddress::try_from("So11111111111111111111111111111111111111112").ok()?;
        let slippage = RuleSlippageBps::from_pct_str("1.5").ok()?;
        Some(SnipeRule::new(
            address,
            RuleSolAmount::new(Lamports::new(1_000_000_000)),
            RuleSolAmount::new(Lamports::new(100_000_000)),
            slippage,
        ))
    }

    #[test]
    fn stores_hot_and_cold_parts() {
        let rule = build_rule();
        assert!(rule.is_some());

        if let Some(rule) = rule {
            assert_eq!(
                rule.address().as_str(),
                "So11111111111111111111111111111111111111112"
            );
            assert_eq!(rule.snipe_height().as_lamports().as_u64(), 1_000_000_000);
            assert_eq!(rule.jito_tip().as_lamports().as_u64(), 100_000_000);
            assert_eq!(rule.slippage().as_bps(), 150);
        }
    }

    #[test]
    fn formats_log_line() {
        let rule = build_rule();
        assert!(rule.is_some());

        if let Some(rule) = rule {
            let line = rule.as_log_line("Token address");
            assert!(line.contains("Token address"));
            assert!(line.contains("Snipe height: 1 SOL"));
            assert!(line.contains("Jito tip: 0.1 SOL"));
            assert!(line.contains("Slippage: 1.50 %"));
        }
    }
}
