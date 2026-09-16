# Media Factory 渲染 worker 镜像
#
# 用途：在算力机上以容器方式运行动态解释画面渲染（HyperFrames + Chromium + ffmpeg），
#      不改动宿主环境（宿主只需 Docker 本体）。
#
# 构建（需能访问 Docker Hub 的机器，构建出 amd64 镜像）：
#   docker build --platform linux/amd64 -t mf-render:0.2.3 .
#
# 导出并部署到算力机：
#   docker save mf-render:0.2.3 | ssh <算力机> 'sudo docker load'
#
# 运行（见 README.md）：
#   docker run -d --name mf-render --restart unless-stopped -p 7788:7788 \
#     -v /srv/mf-render:/data -e HTTP_PROXY= -e HTTPS_PROXY= mf-render:0.2.3

FROM ubuntu:24.04

# 关键：清除代理注入。
# 算力机的 docker 守护进程配置了 HTTP_PROXY，docker 会把它注入容器；若该代理不可用，
# 容器内 apt/npm/curl 全部失败。这里显式清空，避免继承宿主的失效代理。
ENV HTTP_PROXY= HTTPS_PROXY= http_proxy= https_proxy= NO_PROXY=*

# apt 源：默认使用 Ubuntu 官方源（构建机可直连）。
# 如需在国内直连构建，可自行替换为镜像源（注意：基础镜像尚无 ca-certificates，
# 若换 https 源需先装 ca-certificates，或改用 http 源）。

# 基础工具 + ffmpeg（含 libass，用于字幕烧录）+ 中文字体 + Chromium 运行库
RUN set -eux; \
    apt-get update; \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl xz-utils unzip tini \
      ffmpeg \
      fonts-noto-cjk \
      libnss3 libnspr4 libatk1.0-0t64 libatk-bridge2.0-0t64 libcups2t64 \
      libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 \
      libgbm1 libpango-1.0-0 libcairo2 libasound2t64 libatspi2.0-0t64 libxshmfence1; \
    rm -rf /var/lib/apt/lists/*

# Node 22（自包含，来自 npmmirror，版本钉住）
ARG NODE_VERSION=22.22.1
RUN set -eux; \
    curl -fsSL -o /tmp/node.tar.xz \
      "https://cdn.npmmirror.com/binaries/node/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz"; \
    mkdir -p /opt/node; \
    tar -xJf /tmp/node.tar.xz -C /opt/node --strip-components=1; \
    rm /tmp/node.tar.xz; \
    /opt/node/bin/node -v
ENV PATH=/opt/node/bin:$PATH

# HyperFrames 渲染引擎 + GSAP（本地化，避免 CDN 依赖）
ARG HYPERFRAMES_VERSION=0.8.41
ARG GSAP_VERSION=3.14.2
RUN set -eux; \
    mkdir -p /opt/mf-render; \
    cd /opt/mf-render; \
    npm init -y >/dev/null; \
    npm install --registry=https://registry.npmmirror.com --no-audit --no-fund \
      "hyperframes@${HYPERFRAMES_VERSION}" "gsap@${GSAP_VERSION}"; \
    npm cache clean --force

# Chromium（chrome-headless-shell，来自 npmmirror 镜像）
ARG CHROME_VERSION=152.0.7977.30
RUN set -eux; \
    curl -fsSL -o /tmp/chrome.zip \
      "https://cdn.npmmirror.com/binaries/chrome-for-testing/${CHROME_VERSION}/linux64/chrome-headless-shell-linux64.zip"; \
    mkdir -p /opt/mf-render/chrome; \
    unzip -q /tmp/chrome.zip -d /opt/mf-render/chrome; \
    rm /tmp/chrome.zip; \
    chmod +x /opt/mf-render/chrome/chrome-headless-shell-linux64/chrome-headless-shell; \
    /opt/mf-render/chrome/chrome-headless-shell-linux64/chrome-headless-shell --version

# media-factory 二进制（render-server 子命令 + ffmpeg 合成）
COPY media-factory /usr/local/bin/media-factory
RUN chmod +x /usr/local/bin/media-factory && /usr/local/bin/media-factory --version

# 渲染引擎环境（固定，避免遥测/更新检查/自动安装导致的挂起）
ENV HYPERFRAMES_BROWSER_PATH=/opt/mf-render/chrome/chrome-headless-shell-linux64/chrome-headless-shell \
    HYPERFRAMES_NO_TELEMETRY=1 \
    HYPERFRAMES_NO_UPDATE_CHECK=1 \
    HYPERFRAMES_NO_AUTO_INSTALL=1 \
    HYPERFRAMES_FFMPEG_PATH=/usr/bin/ffmpeg \
    HYPERFRAMES_FFPROBE_PATH=/usr/bin/ffprobe \
    MF_RENDER_HOME=/data

WORKDIR /data
VOLUME ["/data"]

# tini 作为 init，正确回收 Chromium 子进程
ENTRYPOINT ["/usr/bin/tini", "--"]
# M0 阶段先提供 shell 以便冒烟验证；render-server 实现后改为：
#   CMD ["/usr/local/bin/media-factory", "render-server", "--port", "7788", "--home", "/data"]
CMD ["/bin/bash"]
