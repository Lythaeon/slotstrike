#![no_main]

use libfuzzer_sys::fuzz_target;
use slotstrike::domain::value_objects::sol_amount::parse_positive_sol_str_to_lamports;

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);
    let parsed = parse_positive_sol_str_to_lamports(raw.as_ref());
    if let Some(lamports) = parsed {
        let formatted = lamports.as_sol_string();
        let round_trip = parse_positive_sol_str_to_lamports(&formatted);
        if let Some(round_trip) = round_trip {
            let _ = round_trip.as_u64().saturating_sub(lamports.as_u64());
        }
    }
});
