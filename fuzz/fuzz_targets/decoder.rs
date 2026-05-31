#![no_main]

use cue_rust_encoding::{DecodeOptions, Encoding, decode_bytes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((selector, payload)) = data.split_first() else {
        return;
    };
    let encoding = match selector % 3 {
        0 => Encoding::Json,
        1 => Encoding::Yaml,
        _ => Encoding::Toml,
    };
    let _result = decode_bytes(encoding, payload, DecodeOptions::default());
});
