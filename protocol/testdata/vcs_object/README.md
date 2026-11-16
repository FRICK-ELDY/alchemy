# Spatial VCS object golden vectors (x00c8)

Fixed canonical CBOR bytes (hex) and blake3 ids for Tree / Commit.

Refresh:

```bash
cargo run -p alchemy-protocol --example gen_vcs_golden
```

Then update the `*.cbor.hex` / `*.blake3` files from stdout. Do not hand-edit bytes.
