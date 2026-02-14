#![no_main]

use libfuzzer_sys::fuzz_target;
use sniper::adapters::fpga_feed::decode_dma_payload;

fuzz_target!(|data: &[u8]| {
    let _ = decode_dma_payload(data);
});
