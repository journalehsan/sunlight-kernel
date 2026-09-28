#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

YAZI_VERSION="26.9.1"
YAZI_TAG="v${YAZI_VERSION}"
YAZI_COMMIT="8dd895c695a5950330c2623eb43debf323b60654"
YAZI_ASSET="yazi-x86_64-unknown-linux-musl.zip"
YAZI_ASSET_SHA256="9b9c39decccf8cb0ff53a7d637d38f8a79d93bbd0099f4ea9c619ef6bb392f5d"

TARGET_DIR="$PROJECT_ROOT/target"
ARCHIVE_PATH="${YAZI_RELEASE_ARCHIVE:-$TARGET_DIR/$YAZI_ASSET}"
SOURCE_DIR="${YAZI_SOURCE_DIR:-$TARGET_DIR/yazi-v${YAZI_VERSION}-src}"
MUSL_CC="${YAZI_MUSL_CC:-musl-gcc}"
GUEST_ELF="$TARGET_DIR/x86_64-unknown-linux-musl/release/yazi"
# The exact payload used in the first Phase 1 QEMU run. An explicit cache
# option permits repeatable ABI investigation on hosts without musl-gcc.
YAZI_PHASE1_ELF_SHA256="15af218a823a16da5a8bb16caae27b03bc7c13bdf83d7c875e3d3d61d2028513"

if [[ "${YAZI_USE_CACHED_ET_EXEC:-0}" == 1 ]]; then
	printf '%s  %s\n' "$YAZI_ASSET_SHA256" "$ARCHIVE_PATH" | sha256sum --check
	unzip -tq "$ARCHIVE_PATH" >/dev/null
	unzip -oq "$ARCHIVE_PATH" -d "$TARGET_DIR/yazi-v${YAZI_VERSION}"
	printf '%s  %s\n' "$YAZI_PHASE1_ELF_SHA256" "$GUEST_ELF" | sha256sum --check
	readelf -h "$GUEST_ELF" | rg -q 'Type:.*EXEC'
	echo "Using pinned Phase 1.1 ET_EXEC payload ($YAZI_PHASE1_ELF_SHA256)"
	exit 0
fi

for required in cargo git curl unzip sha256sum readelf; do
	command -v "$required" >/dev/null || {
		echo "missing required tool: $required" >&2
		exit 1
	}
done
command -v "$MUSL_CC" >/dev/null || {
	echo "missing musl C compiler: $MUSL_CC (install musl-gcc or set YAZI_MUSL_CC)" >&2
	exit 1
}

mkdir -p "$TARGET_DIR"
if [[ ! -f "$ARCHIVE_PATH" ]]; then
	if ! curl -fL \
		"https://github.com/sxyazi/yazi/releases/download/$YAZI_TAG/$YAZI_ASSET" \
		-o "$ARCHIVE_PATH"; then
		curl -fL \
			"https://sourceforge.net/projects/yazi.mirror/files/$YAZI_TAG/$YAZI_ASSET/download" \
			-o "$ARCHIVE_PATH"
	fi
fi
printf '%s  %s\n' "$YAZI_ASSET_SHA256" "$ARCHIVE_PATH" | sha256sum --check
unzip -oq "$ARCHIVE_PATH" -d "$TARGET_DIR/yazi-v${YAZI_VERSION}"

if [[ ! -d "$SOURCE_DIR/.git" ]]; then
	git clone --depth 1 --branch "$YAZI_TAG" https://github.com/sxyazi/yazi.git "$SOURCE_DIR"
fi
actual_commit="$(git -C "$SOURCE_DIR" rev-parse HEAD)"
if [[ "$actual_commit" != "$YAZI_COMMIT" ]]; then
	echo "Yazi source commit mismatch: expected $YAZI_COMMIT, got $actual_commit" >&2
	exit 1
fi

env \
	CARGO_TARGET_DIR="$SOURCE_DIR/target" \
	CC="$MUSL_CC" \
	CC_x86_64_unknown_linux_musl="$MUSL_CC" \
	CC_x86_64-unknown-linux-musl="$MUSL_CC" \
	RUSTFLAGS='-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie' \
	cargo build --manifest-path "$SOURCE_DIR/Cargo.toml" --release --locked \
		--target x86_64-unknown-linux-musl --package yazi-fm --bin yazi

mkdir -p "$(dirname "$GUEST_ELF")"
cp "$SOURCE_DIR/target/x86_64-unknown-linux-musl/release/yazi" "$GUEST_ELF"
bash "$SCRIPT_DIR/stamp_helios_elf.sh" "$GUEST_ELF"
readelf -h "$GUEST_ELF" | rg -q 'Type:.*EXEC' || {
	echo "Yazi build did not produce the ET_EXEC image required by the current loader" >&2
	exit 1
}

echo "Yazi v${YAZI_VERSION} source commit: $actual_commit"
echo "Official asset SHA-256: $YAZI_ASSET_SHA256"
echo "Helios ET_EXEC payload: $GUEST_ELF"
sha256sum "$GUEST_ELF"
