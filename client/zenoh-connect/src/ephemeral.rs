//! x0066: on-memory ECDSA P-256 key pair + CSR for world-issued ephemeral mTLS.
//!
//! Guarantees: private key is not written by this module. Non-guarantees: Zenoh
//! may still need short-lived PEM files for connect_certificate paths.

use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};

/// Ephemeral client identity before world signs the CSR.
pub struct EphemeralMtlsIdentity {
    pub csr_pem: String,
    private_key_pem: String,
}

impl EphemeralMtlsIdentity {
    /// Generate key + CSR in memory. CSR CN is a placeholder; world forces role CN.
    pub fn generate() -> Result<Self> {
        let key_pair =
            KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("rcgen generate key")?;
        let private_key_pem = key_pair.serialize_pem();

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "alchemy-ephemeral-request");
        params.distinguished_name = dn;

        let csr = params
            .serialize_request(&key_pair)
            .context("rcgen serialize CSR")?;
        let csr_pem = csr.pem().context("csr pem")?;

        Ok(Self {
            csr_pem,
            private_key_pem,
        })
    }

    pub fn private_key_pem(&self) -> &str {
        &self.private_key_pem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_csr_pem() {
        let id = EphemeralMtlsIdentity::generate().expect("generate");
        assert!(id.csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(id.private_key_pem().contains("BEGIN PRIVATE KEY")
            || id.private_key_pem().contains("BEGIN EC PRIVATE KEY"));
    }
}
