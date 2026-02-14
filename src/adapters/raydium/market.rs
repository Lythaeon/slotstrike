use std::sync::Arc;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{program_error::ProgramError, pubkey::Pubkey};

use crate::MAX_RETRIES;

const MARKET_STATE_LAYOUT_V3_LEN: usize = 388;
const OWN_ADDRESS_START: usize = 13;
const BASE_VAULT_START: usize = 117;
const QUOTE_VAULT_START: usize = 165;
const EVENT_QUEUE_START: usize = 253;
const BIDS_START: usize = 285;
const ASKS_START: usize = 317;

#[derive(Debug, Clone)]
pub struct Market {
    pub program_id: Pubkey,
    pub state: MarketStateLayoutV3,
}

#[derive(Debug, Clone)]
pub struct MarketStateLayoutV3 {
    pub own_address: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub event_queue: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
}

impl MarketStateLayoutV3 {
    fn read_pubkey(bytes: &[u8], start: usize) -> Option<Pubkey> {
        let end = start.checked_add(32)?;
        let key_bytes: [u8; 32] = bytes.get(start..end)?.try_into().ok()?;
        Some(Pubkey::new_from_array(key_bytes))
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != MARKET_STATE_LAYOUT_V3_LEN {
            return None;
        }

        Some(Self {
            own_address: Self::read_pubkey(bytes, OWN_ADDRESS_START)?,
            base_vault: Self::read_pubkey(bytes, BASE_VAULT_START)?,
            quote_vault: Self::read_pubkey(bytes, QUOTE_VAULT_START)?,
            event_queue: Self::read_pubkey(bytes, EVENT_QUEUE_START)?,
            bids: Self::read_pubkey(bytes, BIDS_START)?,
            asks: Self::read_pubkey(bytes, ASKS_START)?,
        })
    }
}

pub async fn get_market_accounts(rpc: &Arc<RpcClient>, market_id: &Pubkey) -> Option<Market> {
    let mut attempts = 0_usize;

    loop {
        let market_account_info = rpc
            .get_account_with_commitment(market_id, CommitmentConfig::confirmed())
            .await;

        match market_account_info {
            Ok(response) => {
                let account = response.value?;
                let state = MarketStateLayoutV3::decode(&account.data)?;
                return Some(Market {
                    program_id: account.owner,
                    state,
                });
            }
            Err(error) => {
                log::debug!("Error getting market accounts: {}", error);
                if attempts >= MAX_RETRIES {
                    return None;
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
        attempts = attempts.saturating_add(1);
    }
}

pub fn get_associated_authority(
    program_id: &Pubkey,
    market_id: &Pubkey,
) -> Result<(Pubkey, u64), ProgramError> {
    let market_bytes = market_id.to_bytes();
    let mut nonce = 0_u64;

    while nonce < 100_u64 {
        let nonce_bytes = nonce.to_le_bytes();
        let seeds_with_nonce: [&[u8]; 3] = [&market_bytes, &nonce_bytes, &[0_u8; 7]];

        if let Some((pubkey, _)) = Pubkey::try_find_program_address(&seeds_with_nonce, program_id) {
            return Ok((pubkey, nonce));
        }

        nonce = nonce.saturating_add(1);
    }

    Err(ProgramError::Custom(1))
}
