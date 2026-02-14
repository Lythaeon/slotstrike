use crate::domain::{aggregates::RuleBook, entities::SnipeRule};

#[derive(Clone, Copy, Debug)]
pub struct MintAddressMatchSpecification<'address> {
    token_address: &'address str,
}

impl<'address> MintAddressMatchSpecification<'address> {
    #[inline(always)]
    pub const fn new(token_address: &'address str) -> Self {
        Self { token_address }
    }

    #[inline(always)]
    pub fn select(self, rule_book: &RuleBook) -> Option<&SnipeRule> {
        rule_book.mint_rule(self.token_address)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DeployerAddressMatchSpecification<'address> {
    deployer_address: &'address str,
}

impl<'address> DeployerAddressMatchSpecification<'address> {
    #[inline(always)]
    pub const fn new(deployer_address: &'address str) -> Self {
        Self { deployer_address }
    }

    #[inline(always)]
    pub fn select(self, rule_book: &RuleBook) -> Option<&SnipeRule> {
        rule_book.deployer_rule(self.deployer_address)
    }
}

#[cfg(test)]
mod tests {
    use super::{DeployerAddressMatchSpecification, MintAddressMatchSpecification};
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
    fn selects_mint_rule_by_address() {
        let mint_rule = build_rule("So11111111111111111111111111111111111111112");
        assert!(mint_rule.is_some());

        if let Some(mint_rule) = mint_rule {
            let book = RuleBook::new(vec![mint_rule], Vec::new());
            let selected =
                MintAddressMatchSpecification::new("So11111111111111111111111111111111111111112")
                    .select(&book);
            assert!(selected.is_some());
        }
    }

    #[test]
    fn selects_deployer_rule_by_address() {
        let deployer_rule = build_rule("11111111111111111111111111111111");
        assert!(deployer_rule.is_some());

        if let Some(deployer_rule) = deployer_rule {
            let book = RuleBook::new(Vec::new(), vec![deployer_rule]);
            let selected =
                DeployerAddressMatchSpecification::new("11111111111111111111111111111111")
                    .select(&book);
            assert!(selected.is_some());
        }
    }
}
