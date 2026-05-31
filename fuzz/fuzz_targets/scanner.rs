#![no_main]

use cue_rust_syntax::{ParseConfig, scan_bytes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _result = scan_bytes("fuzz.cue", data, ParseConfig::default());
});
