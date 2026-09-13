#!/bin/sh
# Download the exact musl OpenWrt SDK used for Flint2 builds.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sdk_name=openwrt-sdk-25.12.5-armsr-armv8_gcc-14.3.0_musl.Linux-x86_64
archive=$sdk_name.tar.zst
url=https://downloads.openwrt.org/releases/25.12.5/targets/armsr/armv8/$archive
expected_sha256=1b0316604a3e820b2b008a1baff3f9dac6716af942bef800930e58c7de98c98b
destination=${1:-$repo_root/.cache/openwrt-sdk}
sdk_root=$destination/$sdk_name
archive_path=$destination/$archive

case "$destination" in
    /*) ;;
    *) destination=$repo_root/$destination; sdk_root=$destination/$sdk_name; archive_path=$destination/$archive ;;
esac

compiler=$sdk_root/staging_dir/toolchain-aarch64_generic_gcc-14.3.0_musl/bin/aarch64-openwrt-linux-musl-gcc
if [ -d "$sdk_root" ]; then
    [ -x "$compiler" ] || {
        echo "Existing SDK is incomplete: $sdk_root" >&2
        exit 1
    }
    echo "SDK already available: $sdk_root"
    exit 0
fi

mkdir -p "$destination"
if [ ! -f "$archive_path" ]; then
    temporary_archive=$archive_path.part.$$
    trap 'rm -f "$temporary_archive"' EXIT HUP INT TERM
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --retry 3 --output "$temporary_archive" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$temporary_archive" "$url"
    else
        echo "Need curl or wget to download the OpenWrt SDK." >&2
        exit 1
    fi
    actual_sha256=$(sha256sum "$temporary_archive" | awk '{print $1}')
    [ "$actual_sha256" = "$expected_sha256" ] || {
        echo "SDK SHA-256 mismatch: $actual_sha256" >&2
        exit 1
    }
    mv "$temporary_archive" "$archive_path"
    trap - EXIT HUP INT TERM
fi

actual_sha256=$(sha256sum "$archive_path" | awk '{print $1}')
[ "$actual_sha256" = "$expected_sha256" ] || {
    echo "Cached SDK SHA-256 mismatch: $actual_sha256" >&2
    exit 1
}

tar --zstd -xf "$archive_path" -C "$destination"
[ -x "$compiler" ] || {
    echo "Extracted SDK is incomplete: $sdk_root" >&2
    exit 1
}
echo "SDK ready: $sdk_root"
