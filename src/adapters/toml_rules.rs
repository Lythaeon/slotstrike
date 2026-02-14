use std::{collections::HashSet, io, str::FromStr};

use solana_sdk::pubkey::Pubkey;

use crate::{
    domain::{
        config::{RuleKind, load_sniper_config_file},
        entities::SnipeRule,
        value_objects::sol_amount::parse_positive_sol_str_to_lamports,
        value_objects::{RuleAddress, RuleSlippageBps, RuleSolAmount},
    },
    ports::rule_repository::RuleRepository,
};

#[derive(Clone, Debug)]
pub struct TomlRuleRepository {
    config_path: String,
}

impl TomlRuleRepository {
    pub const fn new(config_path: String) -> Self {
        Self { config_path }
    }

    fn report_invalid(message: &str, initial: bool) {
        log::error!("{}", message);
        if initial {
            std::process::exit(1);
        }
    }

    fn parse_rule_entry(
        kind: RuleKind,
        address: &str,
        snipe_height_sol: &str,
        tip_budget_sol: &str,
        slippage_pct: &str,
        initial: bool,
    ) -> Option<SnipeRule> {
        let file_type = match kind {
            RuleKind::Mint => "MINTS",
            RuleKind::Deployer => "DEPLOYERS",
        };

        let address = address.trim().to_owned();
        if address.is_empty() {
            Self::report_invalid(&format!("{} > Empty address", file_type), initial);
            return None;
        }

        let snipe_height = match parse_positive_sol_str_to_lamports(snipe_height_sol) {
            Some(value) => RuleSolAmount::new(value),
            None => {
                Self::report_invalid(
                    &format!(
                        "{} > Invalid snipe height '{}' on address {}",
                        file_type, snipe_height_sol, address
                    ),
                    initial,
                );
                return None;
            }
        };

        let jito_tip = match parse_positive_sol_str_to_lamports(tip_budget_sol) {
            Some(value) => RuleSolAmount::new(value),
            None => {
                Self::report_invalid(
                    &format!(
                        "{} > Invalid tip budget '{}' on address {}",
                        file_type, tip_budget_sol, address
                    ),
                    initial,
                );
                return None;
            }
        };

        let slippage = match RuleSlippageBps::from_pct_str(slippage_pct) {
            Ok(value) => value,
            Err(error) => {
                Self::report_invalid(
                    &format!(
                        "{} > Invalid slippage '{}' on address {}: {}",
                        file_type, slippage_pct, address, error
                    ),
                    initial,
                );
                return None;
            }
        };

        if Pubkey::from_str(&address).is_err() {
            Self::report_invalid(
                &format!("{} > Invalid address {}", file_type, address),
                initial,
            );
            return None;
        }

        let address = match RuleAddress::try_from(address) {
            Ok(value) => value,
            Err(error) => {
                Self::report_invalid(&format!("{} > {}", file_type, error), initial);
                return None;
            }
        };

        Some(SnipeRule::new(address, snipe_height, jito_tip, slippage))
    }
}

impl RuleRepository for TomlRuleRepository {
    async fn load_rules(
        &self,
        file_type: &str,
        initial: bool,
    ) -> Result<Vec<SnipeRule>, io::Error> {
        let config = load_sniper_config_file(&self.config_path).map_err(|message| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("config parse failed: {}", message),
            )
        })?;

        let expected_kind = match file_type {
            "MINTS" => RuleKind::Mint,
            "DEPLOYERS" => RuleKind::Deployer,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported rule file type '{}'", file_type),
                ));
            }
        };

        let mut rules = Vec::new();
        let mut seen_addresses = HashSet::new();

        for entry in config
            .rules
            .iter()
            .filter(|rule| rule.kind == expected_kind)
        {
            let parsed_rule = Self::parse_rule_entry(
                expected_kind,
                &entry.address,
                &entry.snipe_height_sol,
                &entry.tip_budget_sol,
                &entry.slippage_pct,
                initial,
            );

            if let Some(rule) = parsed_rule {
                if !seen_addresses.insert(rule.address().clone()) {
                    Self::report_invalid(
                        &format!(
                            "{} > Same address used multiple times {}",
                            file_type,
                            rule.address()
                        ),
                        initial,
                    );
                    continue;
                }

                rules.push(rule);
            }
        }

        Ok(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::TomlRuleRepository;
    use crate::ports::rule_repository::RuleRepository;
    use std::path::PathBuf;
    use tokio::fs;

    #[tokio::test]
    async fn loads_mint_and_deployer_rules_from_toml() {
        let config_path = temp_config_path("toml_rules_load");
        let write_result = fs::write(
            &config_path,
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "direct"
kernel_tcp_bypass = false
kernel_tcp_bypass_engine = "af_xdp"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[[rules]]
kind = "mint"
address = "So11111111111111111111111111111111111111112"
snipe_height_sol = "0.01"
tip_budget_sol = "0.001"
slippage_pct = "1"

[[rules]]
kind = "deployer"
address = "11111111111111111111111111111111"
snipe_height_sol = "0.02"
tip_budget_sol = "0.001"
slippage_pct = "1"
"#,
        )
        .await;
        assert!(write_result.is_ok());

        let repository = TomlRuleRepository::new(config_path.to_string_lossy().into_owned());
        let mint_rules = repository.load_rules("MINTS", false).await;
        let deployer_rules = repository.load_rules("DEPLOYERS", false).await;

        assert!(mint_rules.is_ok());
        assert!(deployer_rules.is_ok());
        if let (Ok(mint_rules), Ok(deployer_rules)) = (mint_rules, deployer_rules) {
            assert_eq!(mint_rules.len(), 1);
            assert_eq!(deployer_rules.len(), 1);
        }

        let cleanup_result = fs::remove_file(&config_path).await;
        assert!(cleanup_result.is_ok());
    }

    fn temp_config_path(prefix: &str) -> PathBuf {
        let file_name = format!(
            "{}_{}.toml",
            prefix,
            crate::domain::events::unix_timestamp_now_ns()
        );
        std::env::temp_dir().join(file_name)
    }
}
