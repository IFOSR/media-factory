#!/usr/bin/env bash
# media-factory Web 服务管理脚本（兼容层）
# 用法: ./serve.sh {start|stop|restart|status}
# 说明：后台管理能力已内置进 media-factory 二进制（serve --stop/--restart/--status），
#       本脚本仅做转发，推荐使用 media-factory serve 直接管理。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${PORT:-8092}"

# 定位二进制：优先项目内的 release 构建，其次 PATH 中的 media-factory
find_bin() {
  if [ -x "$SCRIPT_DIR/target/release/media-factory" ]; then
    echo "$SCRIPT_DIR/target/release/media-factory"
  elif command -v media-factory >/dev/null 2>&1; then
    command -v media-factory
  else
    echo ""
  fi
}

BIN="$(find_bin)"
if [ -z "$BIN" ]; then
  echo "✗ 未找到 media-factory 二进制，请先编译：cargo build --release"
  exit 1
fi

case "${1:-}" in
  start)   exec "$BIN" serve --port "$PORT" ;;
  stop)    exec "$BIN" serve --stop ;;
  restart) exec "$BIN" serve --port "$PORT" --restart ;;
  status)  exec "$BIN" serve --port "$PORT" --status ;;
  *)
    echo "用法: $0 {start|stop|restart|status}"
    echo "  环境变量 PORT 可指定端口（默认 8092）"
    echo "  等同于: media-factory serve [--port \$PORT] [--stop|--restart|--status]"
    exit 1
    ;;
esac
