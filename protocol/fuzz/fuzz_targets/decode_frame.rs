#![no_main]

use alchemy_protocol::decode_limits::decode_frame_checked;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_frame_checked(data);
});
