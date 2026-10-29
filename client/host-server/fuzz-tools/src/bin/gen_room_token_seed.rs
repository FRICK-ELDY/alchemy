//! Write `../fuzz/corpus/room_token_verify/seed_valid.jwt` matching the fixed PEM
//! in `fuzz_targets/room_token_verify.rs` (x004c).
//!
//! ```bash
//! cargo run --bin gen_room_token_seed --manifest-path client/host-server/fuzz-tools/Cargo.toml
//! ```

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;

/// Must stay identical to `fuzz_targets/room_token_verify.rs`.
const FUZZ_RSA_PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCbd3XqoRJIiNZb
hpI2h3xXD8PuNEOP8VyneIlRs73vtGxClL0aJwotp1mfEZ5Q+xrNZPPVxDg999/s
jZsjEbyfOOkWRTIpH9yrXtvjVPpzdMcb84U20jLnTEOQbp3qp/5tdsS0j8KkFJho
Hct+fKj80ESb+flA0pPL7nV94MS0cbK5h/+7E6MRKbOeVmQqAER+4vfsFkyy4viQ
QXBS4hNBsBw0HkDh67WAAG5KPu9ATVBJV1+bAcjU7r5RTvDMj4Pjj/7r1yP5WgWa
Ac/FX90EJD6AWf3ELek8UAPTc96xqqp1qK7BHjwM5XQpMwH14uX8sHQYHfdnGwKZ
QxXrDSMVAgMBAAECggEACer/KTKmijKSlusDm7odJv0LkerLJEfF/QLDTeSdLsmF
k4zOGzjs3Q3n+YN5TwS/SGWoPusCRIUc8l1z+B2Fo+m7SE9YnHIwKiLxJ7RQOE/g
SOqfmmhNVN9TEHKCHV94wNklbOMeoXLqkyUoH3+1PyWhOYsjCS016Q9jdvOWe5q2
H5ZTauUW3NjwNROrJkeJzJbgNJkpPJFeGcFaNXQ0j0CqYSTWL3HTT0yGeIEmKjIp
OEN205VSWsW/xdgNTEEhHXxSWr2qVIoAwMh4YKar+/wlU8m1ncOfedE4XerwmwYs
9SGz3/OTc0oCH7VKnq/q6m8QU5z+uy+rOPGjn4bIeQKBgQDSZD/RjsV+kgTpZ/ID
fc/8glEVidwwTQBFrEMkyNpKFEfYipyD480cVmFqbFWKyg3hBu/qdzoSQyjdaJtg
TsNqazGRk1fm7YGn4yUICm3aMwlkweMMCYM/m1gJlv8Gc93yz43fAjnL1gdkdHrq
ejBnhMcr4j4INWmXjzBu92fNaQKBgQC9KyIF8PZRKHVfAHsm2T4pEUpyJP6PGXfu
1Tq8qhMXnxaoAAhlEEiwXx8MDKygQgn5qmIApphmqa/3JoEHwUM0wbYveoJoa7Tv
vfD4qp6JA7Q5am8AFuAfZOjshGTPoWjkaPYWi1gjuaSobBRM228ftTWdle5SpvDA
X2FyXB22zQKBgHQwevIfF8BzoZTJG+WDuLYpvd/YN6rRC4JuxNHuvmbE1nym6BMK
Xj71EW11kLRKK+oHK73fxElLn6jO2WgRd2gw9LoBljzpE36FOfE72Tj0kI59aQc/
xaT+BFufcQ2+NWef6iirkQGaHJEEptERU+QHdvssQbfEz6CsqkHODhxpAoGBALBh
rmihw9nqgn3hYfLYLth18dc6XUyqb6R2lzzgy8qDoMlmtm0dYNlq5/UBQFhxGwH6
90QqEgHviE5HTZogfnyLfkcI0N1ySBQeI7iIxid1F69NaVBJBBLhsTkPPTxIARiw
MXzV0ooFB92uYMNE+3kyKlrUOJJCbErC5IaQE3ilAoGBALmM8NbaBzDOJ6V8+bCO
J7zkUSeSabiZSy50Bhr/7bpyTKHEF0Zk+R/iRmf6oIKfjogu1q2ZYJGRk4wQVrIb
vDeL9AfQgHfFCLikF3sgw1JeWhdfk0MlHSMBhGqXX4T09wggyNsxE5IsSPhnfwQf
feAMxzvE1Oq1Rfp2UIpcrb8o
-----END PRIVATE KEY-----";

fn main() {
    #[derive(Serialize)]
    struct Cnf {
        spk: String,
    }
    #[derive(Serialize)]
    struct Claims {
        sub: String,
        jti: String,
        room_id: String,
        role: String,
        exp: usize,
        iat: usize,
        iss: String,
        aud: String,
        cnf: Cnf,
    }

    let encoding = EncodingKey::from_rsa_pem(FUZZ_RSA_PRIV_PEM.as_bytes()).expect("enc");
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("fuzz-kid".into());
    let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
    let token = encode(
        &header,
        &Claims {
            sub: "fuzz-sub".into(),
            jti: "fuzz-jti".into(),
            room_id: "local".into(),
            role: "guest".into(),
            exp: 4_102_444_800,
            iat: 1_700_000_000,
            iss: "alchemy-world".into(),
            aud: "alchemy-room".into(),
            cnf: Cnf { spk },
        },
        &encoding,
    )
    .expect("jwt");

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fuzz/corpus/room_token_verify/seed_valid.jwt");
    fs::create_dir_all(out.parent().expect("parent")).expect("mkdir");
    fs::write(&out, token.as_bytes()).expect("write seed_valid.jwt");
    eprintln!("wrote {}", out.canonicalize().unwrap_or(out).display());
}
