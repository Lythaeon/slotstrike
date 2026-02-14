#![no_main]

use libfuzzer_sys::fuzz_target;
use slotstrike::slices::sniper::pool_filter::{
    is_cpmm_candidate_logs, is_openbook_candidate_logs, is_pool_creation_candidate_logs,
    is_pool_creation_dma_payload,
};

fuzz_target!(|data: &[u8]| {
    let _ = is_pool_creation_dma_payload(data);

    let text = String::from_utf8_lossy(data);
    let logs = text.lines().map(|line| line.to_owned()).collect::<Vec<_>>();
    let _ = is_cpmm_candidate_logs(&logs);
    let _ = is_openbook_candidate_logs(&logs);
    let _ = is_pool_creation_candidate_logs(&logs);
});
