# CSQTT core: Flint2/OpenWrt build

Source: `https://github.com/Endlad2/csqtt-core.git`.

Verified source commit:

```
b9791b0dc8221675cba07c4ddaf37972c3365f3b
```

Target router: GL.iNet Flint 2 / OpenWrt 25.12.5, `aarch64_generic`, musl.
The build uses the OpenWrt SDK toolchain, not the host glibc toolchain.

## SDK

Download and verify the exact SDK once. By default it is stored under the
Git-ignored `.cache/openwrt-sdk/` directory:

```
./openwrt/download-openwrt-sdk.sh
```

The pinned archive is OpenWrt 25.12.5 for `armsr/armv8`:

```
https://downloads.openwrt.org/releases/25.12.5/targets/armsr/armv8/openwrt-sdk-25.12.5-armsr-armv8_gcc-14.3.0_musl.Linux-x86_64.tar.zst
SHA-256: 1b0316604a3e820b2b008a1baff3f9dac6716af942bef800930e58c7de98c98b
```

To reuse a shared extracted SDK, pass its path through `SDK_ROOT`:

```
SDK_ROOT=/path/to/openwrt-sdk-25.12.5-armsr-armv8_gcc-14.3.0_musl.Linux-x86_64 \
  ./openwrt/build-musl.sh
```

## Rebuild

From the repository root:

```
./openwrt/build-musl.sh
```

The script builds in a temporary directory, does not alter `rust-client/target`,
and refuses to overwrite an existing output. To write a differently named
artifact:

```
./openwrt/build-musl.sh /absolute/path/to/client
```

The result must be a static, stripped AArch64 ELF with no dynamic interpreter.
Check it before copying to the router:

```
file dist/client-openwrt-25.12.5-aarch64-musl
sha256sum dist/client-openwrt-25.12.5-aarch64-musl
```

Reference artifact from the first successful build of this source revision:

```
SHA-256 387338f1bbf6e6f3d4d95b9512fadd1633441a37aa324e9435578e3759bf5b9c
```

This is only the native CSQTT core. It accepts a TUN file descriptor over an
abstract Unix-domain socket specified by `--tun-uds`; it neither creates a TUN
device nor supplies policy routing or a SOCKS5 listener.

## TUN FD adapter

Build the companion helper separately:

```
./openwrt/build-tun-adapter.sh
```

It creates a named TUN interface and passes only its open file descriptor to
the core. It never assigns addresses, routes, firewall rules or DNS.
