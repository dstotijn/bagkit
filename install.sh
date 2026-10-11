#!/bin/sh
# Installs bagkit from its GitHub releases.
#
#   curl -fsSL https://dstotijn.github.io/bagkit/install.sh | sh
#
# Environment variables:
#   BAGKIT_VERSION      Release tag to install, for example v0.1.0 (default: latest).
#   BAGKIT_INSTALL_DIR  Directory for the binary (default: ~/.local/bin).
#   BAGKIT_DOWNLOAD_URL Base URL of the release files, for mirrors and testing.

set -eu

REPO="dstotijn/bagkit"

err() {
	echo "bagkit install: $*" >&2
	exit 1
}

detect_target() {
	os=$(uname -s)
	arch=$(uname -m)
	case "$os" in
	Darwin)
		# A shell under Rosetta reports x86_64 on Apple silicon; prefer the native build.
		if [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
			arch="arm64"
		fi
		suffix="apple-darwin"
		;;
	Linux) suffix="unknown-linux-musl" ;;
	*) err "unsupported operating system: $os (download a release from https://github.com/$REPO/releases, or run: cargo install --locked bagkit)" ;;
	esac
	case "$arch" in
	arm64 | aarch64) arch="aarch64" ;;
	x86_64 | amd64) arch="x86_64" ;;
	*) err "unsupported architecture: $arch (run: cargo install --locked bagkit)" ;;
	esac
	echo "$arch-$suffix"
}

download() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL --retry 3 -o "$2" "$1" || err "download failed: $1"
	elif command -v wget >/dev/null 2>&1; then
		wget -q -O "$2" "$1" || err "download failed: $1"
	else
		err "curl or wget is required"
	fi
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d ' ' -f 1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d ' ' -f 1
	else
		err "sha256sum or shasum is required to verify the download"
	fi
}

main() {
	version="${BAGKIT_VERSION:-latest}"
	install_dir="${BAGKIT_INSTALL_DIR:-$HOME/.local/bin}"
	target=$(detect_target)
	if [ -n "${BAGKIT_DOWNLOAD_URL:-}" ]; then
		base="$BAGKIT_DOWNLOAD_URL"
	elif [ "$version" = "latest" ]; then
		base="https://github.com/$REPO/releases/latest/download"
	else
		base="https://github.com/$REPO/releases/download/$version"
	fi
	archive="bagkit-$target.tar.gz"

	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT INT TERM

	echo "Downloading bagkit ($version, $target)..."
	download "$base/$archive" "$tmp/$archive"
	download "$base/$archive.sha256" "$tmp/$archive.sha256"

	expected=$(cut -d ' ' -f 1 <"$tmp/$archive.sha256")
	actual=$(sha256 "$tmp/$archive")
	[ "$expected" = "$actual" ] || err "checksum mismatch for $archive (expected $expected, got $actual)"

	tar -xzf "$tmp/$archive" -C "$tmp"
	mkdir -p "$install_dir"
	# Copy next to the destination, then rename, so a running bagkit is replaced atomically.
	cp "$tmp/bagkit-$target/bagkit" "$install_dir/.bagkit.tmp"
	chmod 755 "$install_dir/.bagkit.tmp"
	mv "$install_dir/.bagkit.tmp" "$install_dir/bagkit"

	echo "Installed $("$install_dir/bagkit" --version) to $install_dir/bagkit"
	case ":$PATH:" in
	*":$install_dir:"*) ;;
	*) echo "Add $install_dir to your PATH to run bagkit." ;;
	esac
}

main "$@"
