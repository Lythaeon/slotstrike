#![no_main]

use libfuzzer_sys::fuzz_target;
use sniper::slices::sniper::log_parser::{extract_i64_after_prefix, extract_u64_after_prefix};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let logs = text.lines().map(|line| line.to_owned()).collect::<Vec<_>>();

    let _ = extract_u64_after_prefix(&logs, "vault_0_amount:");
    let _ = extract_u64_after_prefix(&logs, "vault_1_amount:");
    let _ = extract_i64_after_prefix(&logs, "open_time: ");
});
