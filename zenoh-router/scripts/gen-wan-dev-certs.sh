#!/usr/bin/env bash
# Generate a private CA + server cert for WAN zenohd TLS (x0022).
# Usage: ./gen-wan-dev-certs.sh <server-cn-or-ip> [out-dir]
# Does NOT obtain public CA certs (use Let's Encrypt in production when you have a DNS name).
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

rm -f "$OUT/server.csr" "$OUT/ca-cert.srl"
chmod 600 "$OUT/server-key.pem" "$OUT/ca-key.pem"

echo "Wrote:"
echo "  CA (client ZENOH_TLS_ROOT_CA): $OUT/ca-cert.pem"
echo "  server cert:                   $OUT/server-cert.pem"
echo "  server key:                    $OUT/server-key.pem"
echo "Copy server-*.pem to VPS /etc/alchemy/tls/ and point config.wan.json5 at them."
