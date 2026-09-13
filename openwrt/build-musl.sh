#!/bin/sh
# Build the CSQTT core client for Flint2/OpenWrt (aarch64, musl, static).
# Does not modify the source tree; output must not already exist.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sdk_name=openwrt-sdk-25.12.5-armsr-armv8_gcc-14.3.0_musl.Linux-x86_64
sdk_root=${SDK_ROOT:-$repo_root/.cache/openwrt-sdk/$sdk_name}
linker=$sdk_root/staging_dir/toolchain-aarch64_generic_gcc-14.3.0_musl/bin/aarch64-openwrt-linux-musl-gcc
target=aarch64-unknown-linux-musl
output=${1:-$repo_root/dist/client-openwrt-25.12.5-aarch64-musl}

case "$output" in
    /*) ;;
    *) output=$repo_root/$output ;;
esac

[ -x "$linker" ] || {
    echo "OpenWrt musl linker not found: $linker" >&2
    echo "Run: $repo_root/openwrt/download-openwrt-sdk.sh" >&2
    echo "Or set SDK_ROOT to an extracted matching OpenWrt SDK." >&2
    exit 1
}
[ ! -e "$output" ] || {
    echo "Refusing to overwrite existing output: $output" >&2
    exit 1
}

mkdir -p "$(dirname -- "$output")"
build_root=$(mktemp -d "${TMPDIR:-/tmp}/csqtt-openwrt-build.XXXXXX")
trap 'rm -rf "$build_root"' EXIT HUP INT TERM

cd "$repo_root/rust-client"
CARGO_TARGET_DIR=$build_root \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=$linker \
cargo build --locked --release --target "$target"

install -m 0755 "$build_root/$target/release/client" "$output"
file "$output"
sha256sum "$output"
