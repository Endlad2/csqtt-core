# CSQTT TUN adapter: Flint2 rollout plan

Status: attachment test verified on 2026-09-03. No persistent router
configuration was added.

## Verified constraints

- The core is a static musl AArch64 executable and has a Unix TUN-FD interface:
  `--tun-uds <abstract-socket-name>`.
- It does not create an interface itself. It waits for a process to pass an open
  TUN file descriptor via Unix `SCM_RIGHTS`.
- The core emits `TUNCONF:<IPv4>:<DNS>` after the server supplies tunnel
  parameters. It currently logs this value but does not configure the interface.
- Upstream TUN mode returned local port `0` to the server. Flint2 testing showed
  that the server rejects this GETCONF form. The local Flint2 build keeps a
  UDP control socket open in TUN mode and reports its real port; this is a
  narrow compatibility patch, not a routing change.
- Flint2 already has `/dev/net/tun`, `kmod-tun`, `ip-full`, `nft`, `fw4` and
  policy-routing support. No package installation is needed for the first test.
- The current production path uses the `nikki` TUN interface and routing mark
  `0x81` / table `81`. CSQTT must use distinct names, marks and tables.

## Architecture

Add three CSQTT-owned components. None changes Nikki, WAN, LAN, DNS, or
Smart-Routing configuration.

1. `csqtt-tun-adapter` — a small static AArch64 helper, built with the same
   OpenWrt toolchain. It opens `/dev/net/tun`, creates `csqtt0` with
   `IFF_TUN|IFF_NO_PI`, connects to the abstract Unix socket, passes its FD,
   then keeps the FD open. It has no routing or firewall code.
2. `csqtt-core` — the verified upstream core, started with `--tun-uds
   csqtt-tun-v1`. It receives and writes IP packets through `csqtt0`.
3. `csqtt-supervisor` — a minimal OpenWrt `procd` service wrapper. It starts
   the core and adapter in order, captures the structured `CSQTT_EVENTS=1`
   output, validates the received `TUNCONF`, and only then configures the
   CSQTT interface and an opt-in routing table.

Secrets are stored in a root-readable `0600` UCI-compatible runtime file. The
deployment version should add an upstreamable `--credentials-file` option to
the core so that password and VK values never appear in the process argument
list. This patch is separate from the reproducible upstream build above.

## Staged rollout

### 1. Build and local protocol test

- Build `csqtt-tun-adapter` statically with the same SDK toolchain.
- On the workstation, test the helper against a tiny local Unix socket that
  verifies one FD was received; no router involved.
- Cross-check the helper ELF is static AArch64 and record SHA-256.

Gate: helper sends exactly one usable FD and exits cleanly when its peer closes.

### 2. Router attachment without routing

- Copy core and adapter with verified SHA-256.
- Create `csqtt0`, set it administratively up, start the core and pass its FD.
- Keep the core's stdin open under `procd`: the current core treats a closed
  control channel as a stop request. A background SSH command with stdin from
  `/dev/null` is therefore not a valid service launch.
- Do **not** add an IP rule, route, nft rule, firewall zone, DNS setting, or
  dashboard object.
- Wait for active workers and a validated `TUNCONF` event. Store only the
  tunnel address and DNS values under `/var/run/csqtt/`; do not apply DNS.

Gate: core remains active, `csqtt0` exists, and the TUNCONF event is received.
Existing WAN traffic remains on its existing route.

Rollback: stop the CSQTT service and delete `csqtt0`. No persistent network
configuration exists at this stage.

Verified result: the adapter created `csqtt0`, the core logged `TUN FD успешно
получен!`, and both processes remained active. No CSQTT IPv4 route or `ip rule`
was present. Stopping both processes removed the interface; existing WAN
connectivity remained available.

### 3. One-client / one-destination data test

- Allocate a CSQTT-only table, for example `32050`, and a non-overlapping mark,
  for example `0xc5/0xff`. Do not use Nikki's `0x81` or table `81`.
- Add `default dev csqtt0` only to table `32050`.
- Add a temporary nft table that marks packets only when all are true:
  input is `br-lan`, source is one explicitly chosen test client, and
  destination is one explicitly chosen test address. Add matching temporary
  forward accepts for `br-lan -> csqtt0` and the return direction.
- Verify route selection with `ip route get <test-address> mark 0xc5`, then make
  one HTTPS request from that client. Observe core byte counters and ensure
  normal traffic from the same client still uses Nikki/WAN.

Gate: only the nominated flow traverses `csqtt0`; no default route, DNS, or
Smart-Routing state changes globally.

Rollback: remove the temporary nft table, the single rule, and table `32050`,
then stop CSQTT. This is fully independent of current routing.

### 4. Persistent opt-in integration

- Package the core, adapter and init service; keep it disabled by default.
- Add a dedicated firewall include/zone for `csqtt0`, rather than editing
  Nikki-generated nft chains.
- Add named UCI settings for enabled state, interface name, table, mark, test
  client list and domain/IP selection. Generated runtime files remain under
  `/var/run/csqtt`; user-editable settings remain under `/etc/config/csqtt`.
- Before enabling anything, snapshot UCI, routes, rules and the CSQTT package
  version to `/root/codex-backups/`.

Gate: service restarts and firewall reloads preserve ordinary LAN and Nikki
connectivity.

### 5. Actual switch

Only after a successful opt-in test, choose one of these explicitly:

- selected clients and destinations via CSQTT;
- all LAN traffic via CSQTT; or
- CSQTT as a manually selected fallback path.

This choice is intentionally deferred. The core is TUN-based, so it is not a
drop-in Zashboard SOCKS provider. It should not be added to the dashboard as a
SOCKS endpoint.

## Pre-flight checks before every stage

```
[ -c /dev/net/tun ]
ip link show nikki
ip rule show
ip -4 route show default
nft list ruleset
```

If any CSQTT object collides with an existing interface, mark, table, nft table
or UCI section, stop and select a new CSQTT-only value.
