use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{sync::watch, time};

use crate::{
    domain::{aggregates::RuleBook, entities::SnipeRule, value_objects::RuleAddress},
    ports::rule_repository::RuleRepository,
};

const MINT_RULES: &str = "MINTS";
const DEPLOYER_RULES: &str = "DEPLOYERS";

pub async fn load_rulebook<R: RuleRepository>(
    repository: &R,
    initial: bool,
) -> Result<Arc<RuleBook>, std::io::Error> {
    let mint_rules = repository.load_rules(MINT_RULES, initial).await?;
    let deployer_rules = repository.load_rules(DEPLOYER_RULES, initial).await?;

    Ok(Arc::new(RuleBook::new(mint_rules, deployer_rules)))
}

pub struct ConfigSyncService<R: RuleRepository> {
    repository: Arc<R>,
    sender: watch::Sender<Arc<RuleBook>>,
    previous: Arc<RuleBook>,
}

impl<R: RuleRepository + 'static> ConfigSyncService<R> {
    #[expect(
        clippy::missing_const_for_fn,
        reason = "runtime initialization with channels and Arcs"
    )]
    pub fn new(
        repository: Arc<R>,
        sender: watch::Sender<Arc<RuleBook>>,
        previous: Arc<RuleBook>,
    ) -> Self {
        Self {
            repository,
            sender,
            previous,
        }
    }

    pub fn spawn(self) {
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn run(mut self) {
        let mut interval = time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;

            let next = match load_rulebook(self.repository.as_ref(), false).await {
                Ok(value) => value,
                Err(error) => {
                    log::error!("Failed to refresh config files: {}", error);
                    continue;
                }
            };

            if next == self.previous {
                continue;
            }

            report_changes(self.previous.mint_rules(), next.mint_rules(), "MINTS");
            report_changes(
                self.previous.deployer_rules(),
                next.deployer_rules(),
                "DEPLOYERS",
            );

            if self.sender.send(Arc::clone(&next)).is_err() {
                log::warn!("Config listeners dropped. Stopping config sync service.");
                return;
            }

            self.previous = next;
        }
    }
}

fn report_changes(
    old_data: &HashMap<RuleAddress, SnipeRule>,
    new_data: &HashMap<RuleAddress, SnipeRule>,
    config_name: &str,
) {
    for (address, new_rule) in new_data {
        match old_data.get(address) {
            Some(old_rule) if old_rule != new_rule => {
                log::info!(
                    "{} > Updated - {} \\n\t\tOld > Snipe height: {} SOL, Jito tip: {} SOL, Slippage: {} % \\n\t\tNew > Snipe height: {} SOL, Jito tip: {} SOL, Slippage: {} %",
                    config_name,
                    address,
                    old_rule.snipe_height().as_sol_string(),
                    old_rule.jito_tip().as_sol_string(),
                    old_rule.slippage().as_pct_string(),
                    new_rule.snipe_height().as_sol_string(),
                    new_rule.jito_tip().as_sol_string(),
                    new_rule.slippage().as_pct_string(),
                );
            }
            None => {
                log::info!(
                    "{} > Added - {} \\n\t\tValue > Snipe height: {} SOL, Jito tip: {} SOL, Slippage: {} %",
                    config_name,
                    address,
                    new_rule.snipe_height().as_sol_string(),
                    new_rule.jito_tip().as_sol_string(),
                    new_rule.slippage().as_pct_string(),
                );
            }
            _ => {}
        }
    }

    for address in old_data.keys() {
        if !new_data.contains_key(address) {
            log::info!("{} > Removed - {}", config_name, address);
        }
    }
}
