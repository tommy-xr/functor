#!/bin/sh
# install.sh — download the latest `functor` release binary for this platform
# and drop it in ~/.functor/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/tommy-xr/functor/main/install.sh | sh
#
# Overrides (environment variables):
#   FUNCTOR_VERSION      install a specific version/tag (e.g. 0.1.0 or v0.1.0)
#   FUNCTOR_INSTALL_DIR  install location (default: $HOME/.functor/bin)
set -eu

REPO="tommy-xr/functor"
INSTALL_DIR="${FUNCTOR_INSTALL_DIR:-$HOME/.functor/bin}"

fail() { echo "install: $*" >&2; exit 1; }

# --- Detect platform -> release target triple. -------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64)        target="x86_64-apple-darwin" ;;
      *) fail "unsupported macOS architecture: $arch" ;;
    esac ;;
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      *) fail "unsupported Linux architecture: $arch (only x86_64 is published)" ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    fail "on Windows, download functor-<version>-x86_64-pc-windows-msvc.zip from https://github.com/$REPO/releases" ;;
  *) fail "unsupported OS: $os" ;;
esac

# --- Resolve the version tag. ------------------------------------------------
# Prefer an explicit FUNCTOR_VERSION; else the latest release, falling back to
# the newest prerelease (alpha builds ship as prereleases, which /latest skips).
tag_from() {
  curl -sSL "https://api.github.com/repos/$REPO/$1" 2>/dev/null \
    | grep -m1 '"tag_name"' | cut -d'"' -f4
}

if [ -n "${FUNCTOR_VERSION:-}" ]; then
  tag="$FUNCTOR_VERSION"
  case "$tag" in v*) ;; *) tag="v$tag" ;; esac
else
  tag="$(tag_from releases/latest)"
  [ -n "$tag" ] || tag="$(tag_from releases)"
fi
[ -n "$tag" ] || fail "could not determine the latest release (set FUNCTOR_VERSION to pin one)"

version="${tag#v}"
name="functor-${version}-${target}"
url="https://github.com/$REPO/releases/download/${tag}/${name}.tar.gz"

# --- Download, extract, install. ---------------------------------------------
echo "Downloading functor ${version} (${target})..."
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fSL --progress-bar "$url" -o "$tmp/functor.tar.gz" || fail "download failed: $url"
tar -xzf "$tmp/functor.tar.gz" -C "$tmp"

# The archive nests the binary under functor-<version>-<target>/; fall back to
# a search in case the layout ever changes.
bin="$tmp/$name/functor"
[ -f "$bin" ] || bin="$(find "$tmp" -type f -name functor | head -n1)"
[ -n "$bin" ] && [ -f "$bin" ] || fail "functor binary not found in the archive"

mkdir -p "$INSTALL_DIR"
cp "$bin" "$INSTALL_DIR/functor"
chmod 0755 "$INSTALL_DIR/functor"

echo "Installed functor ${version} -> $INSTALL_DIR/functor"
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo "Run 'functor -d my-game init' to get started." ;;
  *)
    echo
    echo "Add it to your PATH, e.g.:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
