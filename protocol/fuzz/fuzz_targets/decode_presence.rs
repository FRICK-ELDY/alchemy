#![no_main]

use alchemy_protocol::decode_limits::decode_presence_checked;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_presence_checked(data);
});
