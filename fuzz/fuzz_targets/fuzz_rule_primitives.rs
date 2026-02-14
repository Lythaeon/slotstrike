#![no_main]

use libfuzzer_sys::fuzz_target;
use slotstrike::domain::value_objects::{RuleAddress, RuleSlippageBps};

fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data);
    let _ = RuleAddress::try_from(input.as_ref());
    let _ = RuleSlippageBps::from_pct_str(input.as_ref());
});
