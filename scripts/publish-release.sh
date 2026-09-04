#!/usr/bin/env bash
# =============================================================
# 发布同步脚本：把 GitHub Release 的四平台资产同步到自建镜像
#
# 用法:
#   ./scripts/publish-release.sh            # 版本号自动读 Cargo.toml
#   ./scripts/publish-release.sh v0.1.2     # 显式指定版本
#
# 环境变量:
#   MF_MIRROR_SSH   镜像服务器 SSH 目标（默认 root@14.103.216.193）
#   MF_MIRROR_DIR   镜像远端目录（默认 /root/media-factory-release）
#
# 前提: GitHub 上已存在对应版本的 Release（打 tag 后由 CI 自动构建）
# =============================================================
set -euo pipefail

REPO="IFOSR/media-factory"
SERVER="${MF_MIRROR_SSH:-root@14.103.216.193}"
REMOTE_DIR="${MF_MIRROR_DIR:-/root/media-factory-release}"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION="v$(grep -m1 '^version' Cargo.toml | sed 's/version *= *"\(.*\)"/\1/')"
fi

TARGETS=(
  media-factory-aarch64-apple-darwin
  media-factory-x86_64-apple-darwin
  media-factory-x86_64-pc-windows-msvc
  media-factory-x86_64-unknown-linux-gnu
)

echo "==> 同步 Release ${VERSION} → ${SERVER}:${REMOTE_DIR}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

for t in "${TARGETS[@]}"; do
  echo "  下载 ${t}.tar.gz"
  curl -fsSL "https://github.com/${REPO}/releases/download/${VERSION}/${t}.tar.gz" -o "$TMP/${t}.tar.gz"
done

( cd "$TMP" && md5 *.tar.gz > md5sums.txt )
echo -n "$VERSION" > "$TMP/VERSION"

ssh -o BatchMode=yes "$SERVER" "mkdir -p '$REMOTE_DIR'"
scp -q "$TMP"/*.tar.gz "$TMP/md5sums.txt" "$TMP/VERSION" "$SERVER:$REMOTE_DIR/"

echo "==> 验证镜像"
ssh -o BatchMode=yes "$SERVER" "cat '$REMOTE_DIR/VERSION'; ls '$REMOTE_DIR'"
echo "==> 完成。install.sh 将优先从镜像下载（auto 模式）。"
