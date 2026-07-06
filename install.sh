#!/usr/bin/env bash
set -euo pipefail

REPO="NKS01X/vitis"
BIN_DIR="${BIN_DIR:-/usr/local/bin}"
VERSION="${VERSION:-latest}"

ARCH="$(uname -m)"
OS="$(uname -s)"

case "${OS}" in
  Linux)  OS_LOOKUP="unknown-linux-gnu" ;;
  Darwin) OS_LOOKUP="apple-darwin"      ;;
  *)      echo "error: unsupported OS ${OS}" >&2; exit 1 ;;
esac

case "${ARCH}" in
  x86_64|amd64) ARCH_LOOKUP="x86_64"       ;;
  aarch64|arm64) ARCH_LOOKUP="aarch64"      ;;
  *)            echo "error: unsupported arch ${ARCH}" >&2; exit 1 ;;
esac

if [ "${VERSION}" = "latest" ]; then
  ASSET_URL="https://github.com/${REPO}/releases/latest/download/vitis-${ARCH_LOOKUP}-${OS_LOOKUP}"
else
  ASSET_URL="https://github.com/${REPO}/releases/download/${VERSION}/vitis-${ARCH_LOOKUP}-${OS_LOOKUP}"
fi

echo "==> Installing Vitis (${ARCH_LOOKUP}-${OS_LOOKUP})"
echo "==> Downloading ${ASSET_URL}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fsSL "${ASSET_URL}" -o "${TMPDIR}/vitis"
chmod +x "${TMPDIR}/vitis"

if ! command -v sudo >/dev/null 2>&1; then
  install -m 0755 "${TMPDIR}/vitis" "${BIN_DIR}/vitis"
else
  sudo install -m 0755 "${TMPDIR}/vitis" "${BIN_DIR}/vitis"
fi

echo "==> Installed to ${BIN_DIR}/vitis"
echo "==> Try: vitis --help"