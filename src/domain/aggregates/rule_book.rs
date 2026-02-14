use std::collections::HashMap;

use crate::domain::{entities::SnipeRule, value_objects::RuleAddress};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleBook {
    mint_rules: HashMap<RuleAddress, SnipeRule>,
    deployer_rules: HashMap<RuleAddress, SnipeRule>,
}

impl RuleBook {
    pub fn new(mints: Vec<SnipeRule>, deployers: Vec<SnipeRule>) -> Self {
        let mint_rules = mints
            .into_iter()
            .map(|rule| (rule.address().clone(), rule))
            .collect::<HashMap<_, _>>();

        let deployer_rules = deployers
            .into_iter()
            .map(|rule| (rule.address().clone(), rule))
            .collect::<HashMap<_, _>>();

        Self {
            mint_rules,
            deployer_rules,
        }
    }

    #[inline(always)]
    pub fn mint_rule(&self, token_address: &str) -> Option<&SnipeRule> {
        self.mint_rules.get(token_address)
    }

    #[inline(always)]
    pub fn deployer_rule(&self, deployer_address: &str) -> Option<&SnipeRule> {
        self.deployer_rules.get(deployer_address)
    }

    pub const fn mint_rules(&self) -> &HashMap<RuleAddress, SnipeRule> {
        &self.mint_rules
    }

    pub const fn deployer_rules(&self) -> &HashMap<RuleAddress, SnipeRule> {
        &self.deployer_rules
    }

    pub fn mint_log_lines(&self) -> Vec<String> {
        let mut rules = self.mint_rules.values().collect::<Vec<_>>();
        rules.sort_by(|left, right| left.address().as_str().cmp(right.address().as_str()));
        rules
            .iter()
            .map(|rule| rule.as_log_line("Token address"))
            .collect::<Vec<_>>()
    }

    pub fn deployer_log_lines(&self) -> Vec<String> {
        let mut rules = self.deployer_rules.values().collect::<Vec<_>>();
        rules.sort_by(|left, right| left.address().as_str().cmp(right.address().as_str()));
        rules
            .iter()
            .map(|rule| rule.as_log_line("Deployer address"))
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::RuleBook;
    use crate::domain::{
        entities::SnipeRule,
        value_objects::{RuleAddress, RuleSlippageBps, RuleSolAmount, sol_amount::Lamports},
    };

    fn build_rule(address: &str) -> Option<SnipeRule> {
        let address = RuleAddress::try_from(address).ok()?;
        let slippage = RuleSlippageBps::from_pct_str("1").ok()?;
        Some(SnipeRule::new(
            address,
            RuleSolAmount::new(Lamports::new(1_000_000_000)),
            RuleSolAmount::new(Lamports::new(100_000_000)),
            slippage,
        ))
    }

    #[test]
    fn indexes_mint_and_deployer_rules() {
        let mint_rule = build_rule("So11111111111111111111111111111111111111112");
        let deployer_rule = build_rule("11111111111111111111111111111111");
        assert!(mint_rule.is_some());
        assert!(deployer_rule.is_some());

        if let (Some(mint_rule), Some(deployer_rule)) = (mint_rule, deployer_rule) {
            let book = RuleBook::new(vec![mint_rule], vec![deployer_rule]);
            assert!(
                book.mint_rule("So11111111111111111111111111111111111111112")
                    .is_some()
            );
            assert!(
                book.deployer_rule("11111111111111111111111111111111")
                    .is_some()
            );
        }
    }

    #[test]
    fn emits_sorted_log_lines() {
        let mint_a = build_rule("11111111111111111111111111111111");
        let mint_b = build_rule("So11111111111111111111111111111111111111112");
        assert!(mint_a.is_some());
        assert!(mint_b.is_some());

        if let (Some(mint_a), Some(mint_b)) = (mint_a, mint_b) {
            let book = RuleBook::new(vec![mint_b, mint_a], Vec::new());
            let lines = book.mint_log_lines();
            assert_eq!(lines.len(), 2);
            assert!(!lines.is_empty());
            if let Some(first_line) = lines.first() {
                assert!(first_line.contains("11111111111111111111111111111111"));
            }
        }
    }
}
