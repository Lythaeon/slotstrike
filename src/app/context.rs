use std::sync::Arc;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;

use crate::domain::value_objects::TxSubmissionMode;

#[derive(Clone)]
pub struct ExecutionContext {
    pub priority_fees: u64,
    pub rpc: Arc<RpcClient>,
    pub keypair: Arc<Keypair>,
    pub tx_submission_mode: TxSubmissionMode,
    pub jito_url: Arc<String>,
}
