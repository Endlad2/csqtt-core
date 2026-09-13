# SEI CERT C treatment

This adapter is written against the applicable SEI CERT C rules in the supplied
2016 edition. It is not a claim of whole-program conformance for Linux, musl,
the OpenWrt SDK, or the CSQTT core.

| Rule | Adapter treatment |
| --- | --- |
| STR31-C / STR32-C | Interface and abstract UDS names use bounded `strnlen()`, reject missing termination or oversized data, and are copied with explicit lengths. |
| INT30-C / INT31-C | Timeout parsing checks `errno`, the full input, permitted range, and conversion range before casting. Retry multiplication is bounded by the validated timeout. |
| ERR33-C | Every security-relevant system/library call has a checked error path. Cleanup failures are reported and affect process status. |
| SIG30-C / SIG31-C | Signal handler only writes `volatile sig_atomic_t`; installation uses checked `sigaction()`. |
| FIO30-C / FIO47-C | Diagnostics use fixed format strings; external values are arguments, never formats. |
| POSIX peer identity | The adapter verifies `SO_PEERCRED` and accepts only a root-owned CSQTT UDS listener before passing the privileged TUN descriptor. |

Build hardening is in `../build-tun-adapter.sh`: C17, FORTIFY, stack protector,
strict warnings and `-Werror`. The target is a static OpenWrt musl AArch64 ELF.
