#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEFAULT_FORMULA_PATH="$REPO_ROOT/packaging/homebrew-tap/Formula/ashlogin.rb"

OWNER="${OWNER:-life2you}"
REPO="${REPO:-AshLogin}"
HOMEPAGE="${HOMEPAGE:-https://github.com/$OWNER/$REPO}"
FORMULA_CLASS="${FORMULA_CLASS:-Ashlogin}"
BINARY_NAME="${BINARY_NAME:-ashlogin}"
ASSET_BASENAME="${ASSET_BASENAME:-ashlogin}"
DESCRIPTION="${DESCRIPTION:-Terminal-first SSH account manager and login launcher for macOS}"
VERSION=""
FORMULA_PATH="$DEFAULT_FORMULA_PATH"
DRY_RUN=0
TMP_FORMULA_PATH=""

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Required command not found: $cmd" >&2
    exit 1
  fi
}

require_command git
require_command curl
require_command shasum

show_help() {
  cat <<EOF
Usage: ./scripts/update-homebrew-formula.sh [options] [version]

Options:
  --output PATH   Write the generated formula to PATH.
  --dry-run       Print the generated formula to stdout instead of writing it.
  --help          Show this help message.

Defaults:
  output path: $DEFAULT_FORMULA_PATH
  version:     inferred from Cargo.toml when omitted
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --output" >&2
        exit 1
      fi
      FORMULA_PATH="$2"
      shift 2
      ;;
    --output=*)
      FORMULA_PATH="${1#*=}"
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --help)
      show_help
      exit 0
      ;;
    --)
      shift
      if [[ $# -gt 0 ]]; then
        VERSION="$1"
        shift
      fi
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      echo "Use --help to see supported options." >&2
      exit 1
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        echo "Unexpected extra argument: $1" >&2
        exit 1
      fi
      VERSION="$1"
      shift
      ;;
  esac
done

if [[ $# -gt 0 ]]; then
  echo "Unexpected extra arguments: $*" >&2
  exit 1
fi

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)}"

if [[ -z "$VERSION" ]]; then
  echo "Failed to detect version from Cargo.toml" >&2
  exit 1
fi

TAG="v$VERSION"
LOCAL_TAG_COMMIT="$(git -C "$REPO_ROOT" rev-parse "$TAG^{}" 2>/dev/null || true)"

if [[ -z "$LOCAL_TAG_COMMIT" ]]; then
  echo "Local tag $TAG does not exist. Create and push the release tag first." >&2
  exit 1
fi

TAG_VERSION="$(
  git -C "$REPO_ROOT" show "$TAG:Cargo.toml" |
    sed -n 's/^version = "\(.*\)"/\1/p' |
    head -n1
)"

if [[ "$TAG_VERSION" != "$VERSION" ]]; then
  echo "Tag $TAG contains Cargo.toml version ${TAG_VERSION:-<unknown>}, expected $VERSION." >&2
  echo "Refusing to generate a formula from a tag whose source version does not match the requested release." >&2
  exit 1
fi

REMOTE_TAG_COMMIT="$(git -C "$REPO_ROOT" ls-remote --tags origin "refs/tags/$TAG^{}" | awk 'NR==1 {print $1}')"
if [[ -z "$REMOTE_TAG_COMMIT" ]]; then
  REMOTE_TAG_COMMIT="$(git -C "$REPO_ROOT" ls-remote --tags origin "refs/tags/$TAG" | awk 'NR==1 {print $1}')"
fi

if [[ -z "$REMOTE_TAG_COMMIT" ]]; then
  echo "Remote tag $TAG not found on origin. Push the tag before updating the formula." >&2
  exit 1
fi

if [[ "$REMOTE_TAG_COMMIT" != "$LOCAL_TAG_COMMIT" ]]; then
  echo "Remote tag $TAG points to $REMOTE_TAG_COMMIT, but local tag points to $LOCAL_TAG_COMMIT." >&2
  echo "Push the corrected tag before updating the formula." >&2
  exit 1
fi

ARM_URL="https://github.com/$OWNER/$REPO/releases/download/$TAG/${ASSET_BASENAME}-aarch64-apple-darwin.tar.gz"
INTEL_URL="https://github.com/$OWNER/$REPO/releases/download/$TAG/${ASSET_BASENAME}-x86_64-apple-darwin.tar.gz"

ARM_SHA256="$(
  curl --fail --silent --show-error --location --retry 3 "$ARM_URL" |
    shasum -a 256 |
    awk '{print $1}'
)"

INTEL_SHA256="$(
  curl --fail --silent --show-error --location --retry 3 "$INTEL_URL" |
    shasum -a 256 |
    awk '{print $1}'
)"

FORMULA_CONTENT="$(cat <<EOF
class $FORMULA_CLASS < Formula
  desc "$DESCRIPTION"
  homepage "$HOMEPAGE"
  version "$VERSION"
  license "MIT"

  on_macos do
    on_arm do
      url "$ARM_URL"
      sha256 "$ARM_SHA256"
    end

    on_intel do
      url "$INTEL_URL"
      sha256 "$INTEL_SHA256"
    end
  end

  def install
    bin.install "$BINARY_NAME"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/$BINARY_NAME --version")
  end
end
EOF
)"

if [[ "$DRY_RUN" == "1" ]]; then
  printf '%s\n' "$FORMULA_CONTENT"
  {
    echo "Dry run only."
    echo "Version:      $VERSION"
    echo "ARM SHA256:   $ARM_SHA256"
    echo "Intel SHA256: $INTEL_SHA256"
    echo "Output:       $FORMULA_PATH"
  } >&2
  exit 0
fi

mkdir -p "$(dirname "$FORMULA_PATH")"
TMP_FORMULA_PATH="$(mktemp "$(dirname "$FORMULA_PATH")/$(basename "$FORMULA_PATH").XXXXXX")"

cleanup() {
  rm -f "$TMP_FORMULA_PATH"
}
trap cleanup EXIT

printf '%s\n' "$FORMULA_CONTENT" > "$TMP_FORMULA_PATH"
mv "$TMP_FORMULA_PATH" "$FORMULA_PATH"

echo "Updated $FORMULA_PATH"
echo "Version:      $VERSION"
echo "ARM SHA256:   $ARM_SHA256"
echo "Intel SHA256: $INTEL_SHA256"
