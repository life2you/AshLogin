#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <version>"
  exit 1
fi

VERSION="$1"
VERSION="${VERSION#v}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FORMULA_PATH="$REPO_ROOT/packaging/homebrew-tap/Formula/ashlogin.rb"
URL="https://github.com/life2you/AshLogin/archive/refs/tags/v${VERSION}.tar.gz"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

ARCHIVE_PATH="$TMP_DIR/ashlogin-v${VERSION}.tar.gz"
curl -L "$URL" -o "$ARCHIVE_PATH"
SHA256="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"

mkdir -p "$(dirname "$FORMULA_PATH")"

cat >"$FORMULA_PATH" <<EOF
class Ashlogin < Formula
  desc "Terminal-first SSH account manager and login launcher for macOS"
  homepage "https://github.com/life2you/AshLogin"
  url "$URL"
  sha256 "$SHA256"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ashlogin --version")
  end
end
EOF

echo "Updated $FORMULA_PATH"
