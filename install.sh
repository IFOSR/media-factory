#!/usr/bin/env bash
# =============================================================
# Media Factory 一键安装脚本
#
# 用法（三选一，都是一条命令）：
#   1) 远程安装（推荐）：
#      curl -fsSL https://raw.githubusercontent.com/IFOSR/media-factory/main/install.sh | bash
#   2) 克隆后安装：
#      git clone https://github.com/IFOSR/media-factory.git && cd media-factory && ./install.sh
#   3) 指定模式 / 下载源：
#      ./install.sh --release   # 仅安装 Release 预编译版
#      ./install.sh --source    # 仅源码编译安装
#      ./install.sh --mirror    # 仅从自建镜像下载（国内友好）
#      ./install.sh --github    # 仅从 GitHub 下载
#   环境变量 MF_MIRROR 可覆盖镜像地址（默认 https://14.103.216.193）
#
# 可选参数：
#   --bin-dir <dir>   安装目录（默认 ~/.media-factory/bin）
#   --version <v>     指定 Release 版本（默认 latest，如 v0.1.0）
# =============================================================
set -euo pipefail

REPO="IFOSR/media-factory"
MODE="auto"          # auto | release | source
DL_SRC="auto"        # auto(镜像优先→GitHub 回退) | mirror | github
BIN_DIR="$HOME/.media-factory/bin"
VERSION="latest"
MIRROR="${MF_MIRROR:-https://14.103.216.193}"
MIRROR_BASE="$MIRROR/media-factory-release"

# ---------- 输出辅助 ----------
info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m ✓ \033[0m%s\n' "$*"; }
warn()  { printf '\033[1;33m ⚠ \033[0m%s\n' "$*"; }
die()   { printf '\033[1;31m ✗ \033[0m%s\n' "$*" >&2; exit 1; }

# ---------- 参数解析 ----------
while [ $# -gt 0 ]; do
  case "$1" in
    --release)  MODE="release"; shift ;;
    --source)   MODE="source";  shift ;;
    --mirror)   DL_SRC="mirror"; MODE="release"; shift ;;
    --github)   DL_SRC="github"; MODE="release"; shift ;;
    --bin-dir)  BIN_DIR="${2:?--bin-dir 需要一个目录参数}"; shift 2 ;;
    --version)  VERSION="${2:?--version 需要一个版本参数，如 v0.1.0}"; shift 2 ;;
    -h|--help)  sed -n '2,20p' "$0"; exit 0 ;;
    *)          die "未知参数: $1（--help 查看用法）" ;;
  esac
done

# ---------- 平台检测 ----------
OS_RAW="$(uname -s)"
ARCH_RAW="$(uname -m)"
case "$OS_RAW" in
  Darwin) TARGET_OS="apple-darwin" ;;
  Linux)  TARGET_OS="unknown-linux-gnu" ;;   # WSL 也走这里，用 Linux 包
  MINGW*|MSYS*|CYGWIN*)
    cat <<'EOF'
 ✗ 检测到 Windows（Git Bash）。请任选其一：
   1) 下载预编译包解压运行 media-factory.exe：
      https://github.com/IFOSR/media-factory/releases/latest
      （文件：media-factory-x86_64-pc-windows-msvc.tar.gz）
   2) 源码安装：安装 Rust（https://rustup.rs）后，在项目目录执行
      cargo build --release
   3) 在 WSL（Ubuntu）中运行本脚本，按 Linux 方式安装
EOF
    exit 1 ;;
  *)      die "不支持的系统: $OS_RAW（当前支持 macOS / Linux / WSL）" ;;
esac
case "$ARCH_RAW" in
  arm64|aarch64) TARGET_ARCH="aarch64" ;;
  x86_64|amd64)  TARGET_ARCH="x86_64" ;;
  *)             die "不支持的架构: $ARCH_RAW" ;;
esac
ASSET="media-factory-${TARGET_ARCH}-${TARGET_OS}.tar.gz"

info "Media Factory 安装器（模式: ${MODE}，平台: ${TARGET_ARCH}-${TARGET_OS}）"

# ---------- 安装后置：PATH ----------
add_path() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) return 0 ;;
  esac
  local rc=""
  if [ -n "${ZSH_VERSION:-}" ] || [ -f "$HOME/.zshrc" ]; then rc="$HOME/.zshrc"
  elif [ -f "$HOME/.bashrc" ]; then rc="$HOME/.bashrc"
  elif [ -f "$HOME/.bash_profile" ]; then rc="$HOME/.bash_profile"
  fi
  if [ -n "$rc" ] && ! grep -qF "\"$BIN_DIR\"" "$rc" 2>/dev/null; then
    printf '\n# media-factory\nexport PATH="%s:$PATH"\n' "$BIN_DIR" >> "$rc"
    ok "已写入 PATH 到 ${rc}（新终端生效）"
  fi
}

# ---------- 依赖提示（不阻断安装） ----------
check_deps() {
  command -v ffmpeg >/dev/null 2>&1 \
    || warn "未检测到 ffmpeg（播客拼接 / 视频合成需要）：brew install ffmpeg 或 sudo apt install ffmpeg"
  command -v pi >/dev/null 2>&1 \
    || warn "未检测到 pi（默认语言模型层；也可在配置向导中改用自定义 OpenAI 兼容 provider）：npm install -g @earendil-works/pi-coding-agent"
}

post_install() {
  chmod +x "$BIN_DIR/media-factory"
  ok "已安装到 ${BIN_DIR}/media-factory"
  "$BIN_DIR/media-factory" --version || true
  add_path
  check_deps
  cat <<'EOF'

下一步：
  1. 配置向导（选模型、填密钥）:   media-factory wizard
  2. 启动 Web 服务（端口 8092，后台运行）: media-factory serve
     （服务管理：media-factory serve --stop / --restart / --status）
  3. CLI 全流程示例:
     media-factory run input.txt --disclaimer --size portrait
EOF
  printf '\n\033[1m提示：\033[0m若当前终端找不到 media-factory 命令，执行 export PATH="%s:$PATH" 或重开终端。\n' "$BIN_DIR"
}

# ---------- 方式一：Release 预编译包（多源：镜像优先 → GitHub 回退） ----------
# 镜像 latest 语义：读镜像上的 VERSION 文件解析具体版本
mirror_version() {
  curl -fsSL --connect-timeout 8 "$MIRROR_BASE/VERSION" 2>/dev/null || true
}

try_download() { # $1=url $2=来源标记 $3=可选 md5
  local url="$1" from="$2" want_md5="${3:-}"
  info "下载 Release（${from}）: ${url}"
  local tmp; tmp="$(mktemp -d)"
  if ! curl -fsSL --connect-timeout 15 "$url" -o "$tmp/pkg.tar.gz"; then
    rm -rf "$tmp"; return 1
  fi
  if [ -n "$want_md5" ]; then
    local got; got="$(md5 -q "$tmp/pkg.tar.gz" 2>/dev/null || md5sum "$tmp/pkg.tar.gz" | awk '{print $1}')"
    if [ "$got" != "$want_md5" ]; then
      warn "镜像包校验失败（md5 不匹配），尝试下一来源"
      rm -rf "$tmp"; return 1
    fi
  fi
  mkdir -p "$BIN_DIR"
  tar -xzf "$tmp/pkg.tar.gz" -C "$BIN_DIR" media-factory
  rm -rf "$tmp"; return 0
}

install_release() {
  # 解析 GitHub 下载路径
  local gh_path
  [ "$VERSION" = "latest" ] && gh_path="releases/latest/download" || gh_path="releases/download/${VERSION}"
  local gh_url="https://github.com/${REPO}/${gh_path}/${ASSET}"

  # 镜像 URL（平铺目录，latest 时读 VERSION）
  local mver="$VERSION" mirror_md5=""
  if [ "$VERSION" = "latest" ]; then
    mver="$(mirror_version)"
    [ -n "$mver" ] || mver="latest"
  fi
  local mirror_url="$MIRROR_BASE/${ASSET}"

  # 镜像 md5（用于校验）
  mirror_md5="$(curl -fsSL --connect-timeout 8 "$MIRROR_BASE/md5sums.txt" 2>/dev/null | grep " ${ASSET}\$" | awk '{print $1}' || true)"

  case "$DL_SRC" in
    mirror)
      try_download "$mirror_url" "镜像 $MIRROR" "$mirror_md5" || return 1
      ;;
    github)
      try_download "$gh_url" "GitHub" "" || return 1
      ;;
    auto)
      # 镜像优先（含 md5 校验）；镜像无该版本或不可达时回退 GitHub
      local want_mirror=true
      if [ "$VERSION" != "latest" ] && [ -z "$mirror_md5" ]; then
        want_mirror=false   # 镜像上没有这个版本的 md5 记录，视为未同步，直接走 GitHub
      fi
      if $want_mirror && try_download "$mirror_url" "镜像 $MIRROR" "$mirror_md5"; then return 0; fi
      try_download "$gh_url" "GitHub" "" || return 1
      ;;
  esac
  return 0
}

# ---------- 方式二：源码编译 ----------
install_source() {
  # 定位源码：脚本在仓库内（./install.sh）则直接编译；远程管道执行则先克隆
  local src_dir
  if [ -f "$(dirname "$0")/Cargo.toml" ]; then
    src_dir="$(cd "$(dirname "$0")" && pwd)"
  else
    command -v git >/dev/null 2>&1 || die "源码安装需要 git"
    src_dir="$(mktemp -d)/media-factory"
    info "克隆仓库…"
    git clone --depth 1 "https://github.com/${REPO}.git" "$src_dir"
  fi

  # Rust 工具链：缺失则自动经 rustup 安装（minimal profile）
  if ! command -v cargo >/dev/null 2>&1; then
    warn "未检测到 Rust 工具链，自动安装 rustup（minimal）…"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck disable=SC1091
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    command -v cargo >/dev/null 2>&1 || die "Rust 安装失败，请手动安装: https://rustup.rs"
  fi

  info "编译（release，首次约需几分钟）…"
  ( cd "$src_dir" && cargo build --release )
  mkdir -p "$BIN_DIR"
  cp "$src_dir/target/release/media-factory" "$BIN_DIR/media-factory"
}

# ---------- 主流程 ----------
case "$MODE" in
  release)
    install_release || die "下载 Release 失败（可能尚无 ${TARGET_ARCH}-${TARGET_OS} 的预编译包）。可改用: ./install.sh --source"
    ;;
  source)
    install_source
    ;;
  auto)
    if install_release; then
      ok "已安装 Release 预编译版"
    else
      warn "Release 下载失败，自动回退到源码编译…"
      install_source
    fi
    ;;
esac

post_install
