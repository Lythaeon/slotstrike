use std::sync::Arc;

use sof_tx::{
    RecentBlockhashProvider, SubmitPlan, TxSubmitClient, adapters::PluginHostTxProviderAdapter,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{hash::Hash, signature::Keypair};
use tokio::sync::Mutex;

use crate::domain::value_objects::TxSubmissionMode;

#[derive(Clone)]
pub struct ExecutionContext {
    pub priority_fees: u64,
    pub rpc: Arc<RpcClient>,
    pub keypair: Arc<Keypair>,
    pub dry_run: bool,
    pub tx_submission_mode: TxSubmissionMode,
    pub jito_url: Arc<String>,
    pub sof_tx_client: Option<Arc<Mutex<TxSubmitClient>>>,
    pub sof_tx_plan: Option<SubmitPlan>,
    pub sof_tx_uses_jito: bool,
    pub sof_tx_blockhash_adapter: Option<Arc<PluginHostTxProviderAdapter>>,
    pub require_local_blockhash: bool,
}

impl ExecutionContext {
    pub async fn latest_swap_blockhash(&self) -> Result<Hash, String> {
        if let Some(adapter) = &self.sof_tx_blockhash_adapter {
            let blockhash = adapter.latest_blockhash();
            if let Some(blockhash) = blockhash {
                return Ok(Hash::new_from_array(blockhash));
            }
            if self.require_local_blockhash {
                return Err("SOF local recent blockhash is not available yet".to_owned());
            }
        }

        self.rpc
            .get_latest_blockhash()
            .await
            .map_err(|error| format!("failed to fetch blockhash from RPC: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sof::framework::{ObservedRecentBlockhashEvent, ObserverPlugin};
    use sof_tx::adapters::PluginHostTxProviderAdapter;
    use solana_client::nonblocking::rpc_client::RpcClient;
    use solana_sdk::{hash::Hash, signature::Keypair};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::ExecutionContext;
    use crate::domain::value_objects::TxSubmissionMode;

    #[tokio::test]
    async fn latest_swap_blockhash_prefers_sof_adapter_when_available() {
        let expected = [9_u8; 32];
        let adapter = Arc::new(PluginHostTxProviderAdapter::topology_only(
            Default::default(),
        ));
        adapter
            .on_recent_blockhash(ObservedRecentBlockhashEvent {
                slot: 123,
                recent_blockhash: expected,
                dataset_tx_count: 1,
                provider_source: None,
            })
            .await;

        let context = execution_context(
            Arc::new(RpcClient::new("http://127.0.0.1:1".to_owned())),
            Some(adapter),
            false,
        );

        let blockhash = context.latest_swap_blockhash().await;

        assert_eq!(blockhash, Ok(Hash::new_from_array(expected)));
    }

    #[tokio::test]
    async fn latest_swap_blockhash_requires_local_value_for_private_shreds() {
        let context = execution_context(
            Arc::new(RpcClient::new("http://127.0.0.1:1".to_owned())),
            Some(Arc::new(PluginHostTxProviderAdapter::topology_only(
                Default::default(),
            ))),
            true,
        );

        let blockhash = context.latest_swap_blockhash().await;

        assert_eq!(
            blockhash,
            Err("SOF local recent blockhash is not available yet".to_owned())
        );
    }

    #[tokio::test]
    async fn latest_swap_blockhash_falls_back_to_rpc_when_local_is_optional() {
        let expected = Hash::new_from_array([7_u8; 32]);
        let server = spawn_mock_blockhash_rpc(expected).await;
        assert!(server.is_ok());
        let (rpc_url, server) = match server {
            Ok(value) => value,
            Err(_error) => return,
        };
        let context = execution_context(
            Arc::new(RpcClient::new(rpc_url)),
            Some(Arc::new(PluginHostTxProviderAdapter::topology_only(
                Default::default(),
            ))),
            false,
        );

        let blockhash = context.latest_swap_blockhash().await;

        assert_eq!(blockhash, Ok(expected));
        let server_result = server.await;
        assert!(server_result.is_ok());
    }

    fn execution_context(
        rpc: Arc<RpcClient>,
        adapter: Option<Arc<PluginHostTxProviderAdapter>>,
        require_local_blockhash: bool,
    ) -> ExecutionContext {
        ExecutionContext {
            priority_fees: 1,
            rpc,
            keypair: Arc::new(Keypair::new()),
            dry_run: true,
            tx_submission_mode: TxSubmissionMode::Direct,
            jito_url: Arc::new("https://jito.example".to_owned()),
            sof_tx_client: None,
            sof_tx_plan: None,
            sof_tx_uses_jito: false,
            sof_tx_blockhash_adapter: adapter,
            require_local_blockhash,
        }
    }

    async fn spawn_mock_blockhash_rpc(
        expected: Hash,
    ) -> Result<(String, tokio::task::JoinHandle<()>), String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("failed to bind test rpc listener: {error}"))?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| format!("failed to read test rpc listener addr: {error}"))?;

        let server = tokio::spawn(async move {
            let accept_result = listener.accept().await;
            assert!(accept_result.is_ok());
            let Ok((mut stream, _)) = accept_result else {
                return;
            };

            let mut buffer = [0_u8; 4_096];
            let read_result = stream.read(&mut buffer).await;
            assert!(read_result.is_ok());

            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "result": {
                    "context": { "slot": 321_u64 },
                    "value": {
                        "blockhash": expected.to_string(),
                        "lastValidBlockHeight": 654_u64
                    }
                },
                "id": 1_u64
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );

            let write_result = stream.write_all(response.as_bytes()).await;
            assert!(write_result.is_ok());
        });

        Ok((format!("http://{local_addr}"), server))
    }
}
