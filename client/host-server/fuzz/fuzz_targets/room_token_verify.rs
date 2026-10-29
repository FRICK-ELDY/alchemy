//! Fuzz `RoomTokenVerifier::verify` against a fixed in-memory JWKS (x004c / P3-9).
//!
//! Guarantees: arbitrary byte strings as tokens never panic the verify path.
//! Non-guarantees: HTTP JWKS / credential worker; finding cryptographically forged tokens.

#![no_main]

use std::sync::OnceLock;

use base64::Engine;
use host_server::room_token::RoomTokenVerifier;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use libfuzzer_sys::fuzz_target;
use serde::Serialize;

/// Fuzz-only RSA keypair (never a production key). Fixed so CI smoke spends time
/// mutating inputs, not generating 2048-bit primes.
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

const FUZZ_RSA_PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAm3d16qESSIjWW4aSNod8
Vw/D7jRDj/Fcp3iJUbO977RsQpS9GicKLadZnxGeUPsazWTz1cQ4Pfff7I2bIxG8
nzjpFkUyKR/cq17b41T6c3THG/OFNtIy50xDkG6d6qf+bXbEtI/CpBSYaB3Lfnyo
/NBEm/n5QNKTy+51feDEtHGyuYf/uxOjESmznlZkKgBEfuL37BZMsuL4kEFwUuIT
QbAcNB5A4eu1gABuSj7vQE1QSVdfmwHI1O6+UU7wzI+D44/+69cj+VoFmgHPxV/d
BCQ+gFn9xC3pPFAD03PesaqqdaiuwR48DOV0KTMB9eLl/LB0GB33ZxsCmUMV6w0j
FQIDAQAB
-----END PUBLIC KEY-----";

struct Fixture {
    verifier: RoomTokenVerifier,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let encoding = EncodingKey::from_rsa_pem(FUZZ_RSA_PRIV_PEM.as_bytes()).expect("enc");
        let decoding = DecodingKey::from_rsa_pem(FUZZ_RSA_PUB_PEM.as_bytes()).expect("dec");
        let verifier = RoomTokenVerifier::new("http://127.0.0.1:9", "local");
        verifier.insert_decoding_key_for_test("fuzz-kid", decoding);

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
        // Far-future exp so the accept-path smoke does not depend on wall clock drift.
        let exp = 4_102_444_800usize; // 2100-01-01 UTC
        let iat = 1_700_000_000usize;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("fuzz-kid".into());
        let spk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
        let valid_token = encode(
            &header,
            &Claims {
                sub: "fuzz-sub".into(),
                jti: "fuzz-jti".into(),
                room_id: "local".into(),
                role: "guest".into(),
                exp,
                iat,
                iss: "alchemy-world".into(),
                aud: "alchemy-room".into(),
                cnf: Cnf { spk },
            },
            &encoding,
        )
        .expect("jwt");
        // Accept-path smoke once per process. Corpus also ships `seed_valid.jwt`
        // (regen: host-server/fuzz-tools) so libFuzzer mutates near success.
        assert!(
            verifier.verify(&valid_token).is_ok(),
            "fixed PEM fixture must accept its own token"
        );
        Fixture { verifier }
    })
}

fuzz_target!(|data: &[u8]| {
    let f = fixture();
    // UTF-8 lossy keeps binary mutations in play without rejecting early.
    let token = String::from_utf8_lossy(data);
    let _ = f.verifier.verify(token.as_ref());
});
