#!/usr/bin/env bash
# media-factory Web 服务管理脚本
# 用法: ./serve.sh {start|stop|restart|status}

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${PORT:-8092}"
PID_FILE="/tmp/media-factory-serve.pid"
LOG_DIR="$SCRIPT_DIR/logs"
LOG_FILE="$LOG_DIR/serve.log"

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

is_running() {
  [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

start() {
  if is_running; then
    echo "✓ media-factory 已在运行 (PID $(cat "$PID_FILE"))，端口 $PORT"
    return 0
  fi

  BIN="$(find_bin)"
  if [ -z "$BIN" ]; then
    echo "✗ 未找到 media-factory 二进制，请先编译：cargo build --release"
    return 1
  fi

  # 清理残留的失效 PID 文件
  rm -f "$PID_FILE"
  mkdir -p "$LOG_DIR"
  echo "启动 media-factory serve --port $PORT ..."
  nohup "$BIN" serve --port "$PORT" >> "$LOG_FILE" 2>&1 &
  echo $! > "$PID_FILE"

  # 等待进程就绪
  for _ in $(seq 1 20); do
    if is_running; then
      # 探测端口是否已监听
      if (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null; then
        exec 3>&- 3<&- 2>/dev/null || true
        echo "✓ 已启动 (PID $(cat "$PID_FILE"))，地址 http://localhost:$PORT"
        echo "  日志: $LOG_FILE"
        return 0
      fi
    fi
    sleep 0.3
  done

  if is_running; then
    echo "✓ 进程已启动 (PID $(cat "$PID_FILE"))，但端口探测未通过，请查看日志: $LOG_FILE"
    return 0
  else
    echo "✗ 启动失败，查看日志: $LOG_FILE"
    rm -f "$PID_FILE"
    return 1
  fi
}

stop() {
  if [ ! -f "$PID_FILE" ]; then
    echo "未找到 PID 文件，服务未在运行"
    return 0
  fi

  pid="$(cat "$PID_FILE")"
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    # 等待进程退出
    for _ in $(seq 1 20); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.3
    done
    # 仍存活则强杀
    if kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || true
    fi
    echo "✓ 已停止 (PID $pid)"
  else
    echo "PID $pid 已不存在（进程可能早已退出）"
  fi
  rm -f "$PID_FILE"
}

status() {
  if is_running; then
    echo "● 运行中 (PID $(cat "$PID_FILE"))，端口 $PORT → http://localhost:$PORT"
  else
    echo "○ 未运行"
  fi
}

case "${1:-}" in
  start)   start ;;
  stop)    stop ;;
  restart) stop; start ;;
  status)  status ;;
  *)
    echo "用法: $0 {start|stop|restart|status}"
    echo "  环境变量 PORT 可指定端口（默认 8092）"
    exit 1
    ;;
esac
