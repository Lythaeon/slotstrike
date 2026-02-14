use crate::adapters::raydium::{RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID};

const CPMM_EXCLUSION_MARKERS: [&str; 6] = [
    "SwapBaseIn",
    "SwapBaseOutput",
    "CollectProtocolFee",
    "Deposit",
    "CollectFundFee",
    "Burn",
];

#[inline(always)]
pub fn is_cpmm_candidate_logs(logs: &[String]) -> bool {
    logs.iter()
        .any(|line| line.contains(RAYDIUM_STANDARD_AMM_PROGRAM_ID))
        && !logs.iter().any(|line| {
            CPMM_EXCLUSION_MARKERS
                .iter()
                .any(|marker| line.contains(marker))
        })
}

#[inline(always)]
pub fn is_openbook_candidate_logs(logs: &[String]) -> bool {
    logs.iter().any(|line| line.contains(RAYDIUM_V4_PROGRAM_ID))
        && logs.iter().any(|line| line.contains("initialize2"))
}

#[inline(always)]
pub fn is_pool_creation_candidate_logs(logs: &[String]) -> bool {
    is_cpmm_candidate_logs(logs) || is_openbook_candidate_logs(logs)
}

#[inline(always)]
pub fn is_pool_creation_dma_payload(payload: &[u8]) -> bool {
    let cpmm_hit = payload_contains(payload, RAYDIUM_STANDARD_AMM_PROGRAM_ID.as_bytes());
    let openbook_hit = payload_contains(payload, RAYDIUM_V4_PROGRAM_ID.as_bytes())
        && payload_contains(payload, b"initialize2");

    if openbook_hit {
        return true;
    }

    if !cpmm_hit {
        return false;
    }

    !CPMM_EXCLUSION_MARKERS
        .iter()
        .any(|marker| payload_contains(payload, marker.as_bytes()))
}

#[inline(always)]
fn payload_contains(payload: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }

    payload.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::{is_pool_creation_candidate_logs, is_pool_creation_dma_payload};
    use crate::adapters::raydium::RAYDIUM_V4_PROGRAM_ID;

    #[test]
    fn matches_openbook_logs() {
        let logs = vec![
            format!("Program {} invoke [1]", RAYDIUM_V4_PROGRAM_ID),
            "Program log: initialize2".to_owned(),
        ];
        assert!(is_pool_creation_candidate_logs(&logs));
    }

    #[test]
    fn matches_openbook_dma_payload() {
        let payload = format!(
            "log=Program {}\nlog=Program log: initialize2",
            RAYDIUM_V4_PROGRAM_ID
        );
        assert!(is_pool_creation_dma_payload(payload.as_bytes()));
    }
}
