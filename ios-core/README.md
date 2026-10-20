# csqtt-ios-core

iOS static library core for CSQTT, synced with `rust-client` **2.5.0**.

## What changed in 2.5.0

- **New modules** wired through `lib.rs`:
  - `direct_relay` (smoltcp-based direct traffic bypass + DNS relay)
  - `srtp_mode` (WRAP-SRTP / DTLS-SRTP / AUTO selection)
  - `turn_ip_pool` (custom TURN IP rotation with failure cooldown)
  - `turn_stream` (TURN over TCP/TLS)
  - `vk_js_calls` (Auto JS credential broker)
  - `sni` (domain/IP routing rules)
  - `shared/dtls`, `shared/spsc_ring` re-exported via `shared.rs`
- **`RuntimeParams`** updated: added `turn_ip_pool`, `client_wrap_key`, `srtp_mode`; `workers` is now `Arc<AtomicUsize>`; added `configured_workers`.
- **`Dispatcher`**: now started with `Dispatcher::start_with_domain_routing` (takes `DomainRoutingConfig`, `worker_count`).
- **`CsqttConfig`** gained fields: `srtp_mode`, `custom_turn_ips`, `domain_routing`, `exclude_ru_zones`, `template_domains`, `domain_whitelist`, `direct_domains`, `ip_routing`, `ip_whitelist`, `direct_ips`, `exact_workers`.
- **`CsqttConfig::to_args_vec`** and **`csqtt_run`** C-ABI signature extended accordingly.
- **`Cargo.toml`**: added `flate2`, `rcgen`, `futures`, `netstack-smoltcp`, `x25519-dalek`; `netstack-smoltcp` patched to vendor path.

## C-ABI

The exported `csqtt_run` function now takes 30 arguments (was 20). Kotlin/Swift callers must be updated to pass the new parameters (in the same order as the CLI flags):

```

peer, vk, password, listen, workers, device_id, vk_hash_mode, vk_auth_mode,
captcha_mode, fingerprint, client_ids, obfs, turn_transport, srtp_mode,
generation, salt, turn, custom_turn_ips, port, allow_hash_redistribution,
validate_vk_hashes, tun_uds, domain_routing, exclude_ru_zones,
template_domains, domain_whitelist, direct_domains, ip_routing,
ip_whitelist, direct_ips, exact_workers

```

All `*const c_char` args tolerate NULL (treated as empty string).

## Build

```bash
cargo build --release --target aarch64-apple-ios --profile ios-release
```

The output `libcsqtt_ios_core.a` can be linked into the iOS app.

## Notes

- `run_client` no longer handles `validate_vk_hashes` inline (kept for API compat but not implemented in core — validation is done by the app layer via the CLI `--validate-vk-hashes` path in `rust-client/main.rs`).
- `ready_credential_tx` is `None` in the iOS core (Auto JS broker path is not used inside the static lib; Auto JS is handled by the Kotlin side and credentials are passed via `client_ids`).

