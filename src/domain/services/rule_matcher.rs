use std::sync::Arc;

use crate::domain::{
    aggregates::RuleBook,
    entities::{SnipeRuleCold, SnipeRuleHot},
    specifications::{DeployerAddressMatchSpecification, MintAddressMatchSpecification},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleSource {
    Mint,
    Deployer,
}

#[derive(Clone, Debug)]
pub struct MatchedRule {
    pub source: RuleSource,
    pub hot: SnipeRuleHot,
    pub cold: Arc<SnipeRuleCold>,
}

pub struct RuleMatcher;

impl RuleMatcher {
    #[inline(always)]
    pub fn match_rule(
        rule_book: &RuleBook,
        token_address: &str,
        deployer_address: &str,
    ) -> Option<MatchedRule> {
        let mint_specification = MintAddressMatchSpecification::new(token_address);
        if let Some(rule) = mint_specification.select(rule_book) {
            return Some(MatchedRule {
                source: RuleSource::Mint,
                hot: rule.hot(),
                cold: rule.cold_arc(),
            });
        }

        let deployer_specification = DeployerAddressMatchSpecification::new(deployer_address);
        deployer_specification
            .select(rule_book)
            .map(|rule| MatchedRule {
                source: RuleSource::Deployer,
                hot: rule.hot(),
                cold: rule.cold_arc(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{RuleMatcher, RuleSource};
    use crate::domain::{
        aggregates::RuleBook,
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
    fn prioritizes_mint_match_over_deployer_match() {
        let mint = build_rule("So11111111111111111111111111111111111111112");
        let deployer = build_rule("11111111111111111111111111111111");
        assert!(mint.is_some());
        assert!(deployer.is_some());

        if let (Some(mint), Some(deployer)) = (mint, deployer) {
            let book = RuleBook::new(vec![mint], vec![deployer]);
            let matched = RuleMatcher::match_rule(
                &book,
                "So11111111111111111111111111111111111111112",
                "11111111111111111111111111111111",
            );
            assert!(matched.is_some());
            if let Some(matched) = matched {
                assert_eq!(matched.source, RuleSource::Mint);
            }
        }
    }

    #[test]
    fn falls_back_to_deployer_match() {
        let deployer = build_rule("11111111111111111111111111111111");
        assert!(deployer.is_some());

        if let Some(deployer) = deployer {
            let book = RuleBook::new(Vec::new(), vec![deployer]);
            let matched = RuleMatcher::match_rule(
                &book,
                "So11111111111111111111111111111111111111112",
                "11111111111111111111111111111111",
            );
            assert!(matched.is_some());
            if let Some(matched) = matched {
                assert_eq!(matched.source, RuleSource::Deployer);
            }
        }
    }
}
