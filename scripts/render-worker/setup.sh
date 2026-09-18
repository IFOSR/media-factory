#!/usr/bin/env bash
# =============================================================
# 渲染 Worker 部署脚本（在算力机上执行）
#
# 用法:
#   ./setup.sh load <image.tar.gz>     # 导入镜像（从构建机传过来）
#   ./setup.sh start                   # 启动/重启渲染容器
#   ./setup.sh status                  # 查看状态与自检
#   ./setup.sh smoke                   # 渲染冒烟测试（20 秒样片）
#   ./setup.sh stop                    # 停止并删除容器（宿主恢复原状）
#
# 环境变量:
#   MF_RENDER_IMAGE   镜像名（默认 mf-render:0.3.4）
#   MF_RENDER_DATA    数据卷目录（默认 /srv/mf-render）
#   MF_RENDER_PORT    监听端口（默认 7788）
#
# 注意：本脚本只做 docker load / run / rm，不修改 docker 守护进程配置，
#      以免影响算力机上其他正在运行的容器（如 vLLM 服务）。
# =============================================================
set -euo pipefail

IMAGE="${MF_RENDER_IMAGE:-mf-render:0.3.4}"
DATA_DIR="${MF_RENDER_DATA:-/srv/mf-render}"
PORT="${MF_RENDER_PORT:-7788}"
CONTAINER="mf-render"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m ✓ \033[0m%s\n' "$*"; }
warn() { printf '\033[1;33m ⚠ \033[0m%s\n' "$*"; }
die()  { printf '\033[1;31m ✗ \033[0m%s\n' "$*" >&2; exit 1; }

docker_ok() {
  command -v docker >/dev/null 2>&1 || die "未找到 docker"
  docker info >/dev/null 2>&1 || sudo -n docker info >/dev/null 2>&1 || die "无法访问 docker（需要权限）"
}

# 统一用 sudo（若当前用户已在 docker 组则不需要）
dk() { if docker info >/dev/null 2>&1; then docker "$@"; else sudo -n docker "$@"; fi; }

cmd_load() {
  local tar="${1:?用法: setup.sh load <image.tar.gz>}"
  [ -f "$tar" ] || die "文件不存在: $tar"
  info "导入镜像 $tar"
  gunzip -c "$tar" | dk load
  ok "镜像已导入"
}

cmd_start() {
  dk image inspect "$IMAGE" >/dev/null 2>&1 || die "镜像 $IMAGE 不存在，请先 setup.sh load"
  mkdir -p "$DATA_DIR"
  info "启动容器 $CONTAINER（镜像 $IMAGE，端口 $PORT，数据卷 $DATA_DIR）"
  dk rm -f "$CONTAINER" >/dev/null 2>&1 || true
  # -e 清空代理：宿主的 docker 守护进程可能注入失效代理，导致容器内网络不可用。
  #   注意：若渲染 worker 需要经代理出网，请改为传入正确的代理地址。
  dk run -d --name "$CONTAINER" --restart unless-stopped \
    -p "${PORT}:${PORT}" \
    -v "${DATA_DIR}:/data" \
    -e HTTP_PROXY= -e HTTPS_PROXY= -e http_proxy= -e https_proxy= \
    "$IMAGE" \
    /usr/local/bin/media-factory render-server --port "$PORT" --home /data
  sleep 2
  cmd_status
}

cmd_status() {
  info "容器状态"
  dk ps -a --filter "name=$CONTAINER" --format '  {{.Names}} | {{.Status}} | {{.Ports}}' || true
  if dk ps --filter "name=$CONTAINER" --format '{{.Names}}' | grep -q "$CONTAINER"; then
    info "环境自检"
    dk exec "$CONTAINER" bash -c '
      echo "  Node:        $(node -v)"
      echo "  ffmpeg:      $(ffmpeg -version | head -1 | cut -d" " -f1-3)"
      echo "  中文字体:     $(fc-list | grep -c CJK) 个"
      echo "  hyperframes: $(/opt/mf-render/node_modules/.bin/hyperframes --version 2>/dev/null | head -1)"
      echo "  media-factory: $(/usr/local/bin/media-factory --version)"' 2>/dev/null || warn "自检失败（容器可能仍在启动）"
    info "健康检查"
    curl -s -m 5 -o /dev/null -w "  http://localhost:${PORT}/health → %{http_code}\n" "http://localhost:${PORT}/health" 2>/dev/null \
      || warn "健康检查未响应（render-server 若尚未实现则属预期）"
    info "资源占用"
    dk stats "$CONTAINER" --no-stream --format '  CPU:{{.CPUPerc}}  MEM:{{.MemUsage}}' 2>/dev/null || true
  else
    warn "容器未运行"
  fi
}

cmd_smoke() {
  info "渲染冒烟测试（需 $DATA_DIR/demo 下存在组合）"
  dk exec "$CONTAINER" bash -c '
    cd /data/demo || { echo "缺少 /data/demo"; exit 1; }
    time /opt/mf-render/node_modules/.bin/hyperframes render -o renders/smoke.mp4 -q looks -f 24 -w 16 2>&1 | tail -3
    ls -la renders/smoke.mp4' || die "冒烟失败"
  ok "冒烟完成"
}

cmd_stop() {
  info "停止并删除容器 $CONTAINER（镜像保留）"
  dk rm -f "$CONTAINER" >/dev/null 2>&1 || true
  ok "已停止。宿主无任何 Node/Chromium/字体残留（仅镜像 ${IMAGE}）"
  echo "  如需彻底清理: docker rmi $IMAGE"
}

case "${1:-}" in
  load)   shift; cmd_load "$@" ;;
  start)  cmd_start ;;
  status) cmd_status ;;
  smoke)  cmd_smoke ;;
  stop)   cmd_stop ;;
  *) sed -n '2,20p' "$0"; exit 1 ;;
esac
