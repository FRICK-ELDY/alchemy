#!/usr/bin/env bash
# Generate a private CA + server cert for WAN zenohd TLS / mTLS (x0022 / x004e).
# Usage: ./gen-wan-dev-certs.sh <server-cn-or-ip> [out-dir]
# Does NOT obtain public CA certs (use Let's Encrypt in production when you have a DNS name).
#
# x004e / x0066: also writes client-ca-*.pem (same CA by default).
# world signs with client-ca-key.pem; zenohd verifies with client-ca-cert.pem.
# Relay Agent fallback (x004e) is the path that also needs the key on the VPS.
set -euo pipefail
CN="${1:?usage: $0 <cn-or-ip> [out-dir]}"
OUT="${2:-./tls-wan-dev}"
mkdir -p "$OUT"

openssl req -x509 -newkey rsa:2048 -nodes -days 825 \
  -keyout "$OUT/ca-key.pem" -out "$OUT/ca-cert.pem" \
  -subj "/CN=alchemy-wan-dev-ca"

openssl req -newkey rsa:2048 -nodes \
  -keyout "$OUT/server-key.pem" -out "$OUT/server.csr" \
  -subj "/CN=${CN}"

# SAN: IPv4 / IPv6 / DNS
if [[ "$CN" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  SAN="IP:${CN}"
elif [[ "$CN" == *:* ]]; then
  # IPv6 literal (may include brackets from callers — strip them)
  IP6="${CN#[}"
  IP6="${IP6%]}"
  SAN="IP:${IP6}"
else
  SAN="DNS:${CN}"
fi

openssl x509 -req -in "$OUT/server.csr" -CA "$OUT/ca-cert.pem" -CAkey "$OUT/ca-key.pem" \
  -CAcreateserial -out "$OUT/server-cert.pem" -days 825 \
  -extfile <(printf "subjectAltName=%s\n" "$SAN")

# Client CA material (dev default: same CA as the server cert).
# world (x0066) signs; zenohd only needs the cert. Relay Agent also needs the key.
cp -f "$OUT/ca-cert.pem" "$OUT/client-ca-cert.pem"
cp -f "$OUT/ca-key.pem" "$OUT/client-ca-key.pem"

rm -f "$OUT/server.csr" "$OUT/ca-cert.srl"
chmod 600 "$OUT/server-key.pem" "$OUT/ca-key.pem" "$OUT/client-ca-key.pem"

echo "Wrote:"
echo "  CA (client ZENOH_TLS_ROOT_CA):     $OUT/ca-cert.pem"
echo "  server cert:                       $OUT/server-cert.pem"
echo "  server key:                        $OUT/server-key.pem"
echo "  client CA cert (zenohd + world):   $OUT/client-ca-cert.pem"
echo "  client CA key (world MTLS_CA_KEY): $OUT/client-ca-key.pem"
echo "VPS /etc/alchemy/tls/: server-cert.pem, server-key.pem, client-ca-cert.pem"
echo "world: client-ca-cert.pem + client-ca-key.pem (MTLS_CA_CERT_PATH / MTLS_CA_KEY_PATH)"
echo "Do NOT re-run after LE cutover if you must keep the same client CA."
echo "See .workspace/0_doc/manual/relay-server-wan.md"
