#!/bin/sh
# Build a static TUN FD adapter for Flint2/OpenWrt.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sdk_name=openwrt-sdk-25.12.5-armsr-armv8_gcc-14.3.0_musl.Linux-x86_64
sdk_root=${SDK_ROOT:-$repo_root/.cache/openwrt-sdk/$sdk_name}
compiler=$sdk_root/staging_dir/toolchain-aarch64_generic_gcc-14.3.0_musl/bin/aarch64-openwrt-linux-musl-gcc
source=$repo_root/openwrt/tun-adapter/csqtt-tun-adapter.c
output=${1:-$repo_root/dist/csqtt-tun-adapter-openwrt-25.12.5-aarch64-musl}

[ -x "$compiler" ] || {
    echo "OpenWrt musl compiler not found: $compiler" >&2
    echo "Run: $repo_root/openwrt/download-openwrt-sdk.sh" >&2
    echo "Or set SDK_ROOT to an extracted matching OpenWrt SDK." >&2
    exit 1
}
[ ! -e "$output" ] || { echo "Refusing to overwrite existing output: $output" >&2; exit 1; }
mkdir -p "$(dirname -- "$output")"
STAGING_DIR=$sdk_root/staging_dir "$compiler" \
    -std=c17 -Os -static -s -D_FORTIFY_SOURCE=2 -fstack-protector-strong \
    -Wall -Wextra -Wconversion -Wsign-conversion -Wshadow -Wstrict-prototypes -fanalyzer \
    -Werror -o "$output" "$source"
file "$output"
sha256sum "$output"
