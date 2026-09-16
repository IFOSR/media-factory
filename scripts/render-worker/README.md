# 渲染 Worker（Docker 隔离方案）

在算力机上以**容器**方式运行动态解释画面渲染（HyperFrames + Chromium + ffmpeg），
**不改动宿主环境**——宿主只需 Docker 本体。适用于同时跑着其他服务（如 vLLM）的算力机。

## 为什么用容器

- 算力机往往已在跑别的服务（我们的 way 上就有 2 个 vLLM 容器），不能污染系统环境
- 渲染栈约 2.7GB（Node + hyperframes 依赖 + Chromium + 中文字体），装进容器便于整体升级/回滚/删除
- 版本钉住（Node 22.22.1 / hyperframes 0.8.41 / Chrome 152），避免依赖漂移

## 镜像内容

| 组件 | 版本 | 用途 |
|---|---|---|
| Ubuntu | 24.04 | 基础 |
| Node.js | 22.22.1（npmmirror tarball） | hyperframes 运行时 |
| hyperframes | 0.8.41 | HTML → 视频渲染引擎 |
| gsap | 3.14.2（本地化） | 动画时间轴（**不用 CDN**） |
| Chrome Headless Shell | 152.0.7977.30（npmmirror） | 逐帧渲染 |
| ffmpeg | 6.1.1（含 libass/drawtext） | 字幕烧录 + 音频合成 |
| fonts-noto-cjk | Ubuntu 包 | 中文渲染 |
| media-factory | 与 Release 同版本 | `render-server` + 合成 |

镜像体积约 **2.71GB**（压缩传输约 1GB）。

## 一、构建（在有 Docker Hub 访问能力的机器上）

```bash
cd scripts/render-worker
# 取对应平台的 media-factory 二进制（示例：Linux x64）
curl -fsSL -o /tmp/mf.tar.gz https://github.com/IFOSR/media-factory/releases/latest/download/media-factory-x86_64-unknown-linux-gnu.tar.gz
tar xzf /tmp/mf.tar.gz -C .            # 得到 ./media-factory
docker build --platform linux/amd64 -t mf-render:0.3.1 .
```

> 构建机需要能访问 Docker Hub（拉 ubuntu:24.04）与 npmmirror（Node/hyperframes/Chromium）。
> 若构建机在境内且直连 Docker Hub 困难，可先解决基础镜像获取（镜像加速器或从别处导入）。

## 二、部署到算力机

```bash
docker save mf-render:0.3.1 | gzip -1 | ssh <算力机> 'gunzip | sudo docker load'
```

实测：2.71GB 镜像经局域网传输约 **1 分 14 秒**。

## 三、启动容器

```bash
ssh <算力机> '
sudo docker rm -f mf-render 2>/dev/null || true
sudo docker run -d --name mf-render --restart unless-stopped \
  -p 7788:7788 \
  -v /srv/mf-render:/data \
  -e HTTP_PROXY= -e HTTPS_PROXY= -e http_proxy= -e https_proxy= \
  mf-render:0.3.1 \
  /usr/local/bin/media-factory render-server --port 7788 --home /data
'
```

也可用 `setup.sh` 一键完成加载镜像 + 启动 + 自检。

### ⚠️ 两个必须注意的点

1. **`-e HTTP_PROXY=` 等清空代理是必须的**
   算力机的 docker 守护进程可能配置了代理（我们的 way 上是 `http-proxy.conf` 指向一台离线机器）。
   Docker 会把守护进程的代理环境变量注入容器，导致容器内 curl/HTTP 请求全部失败。
   镜像内也已 `ENV HTTP_PROXY=` 兜底，但启动时显式清空更稳妥。

2. **不要重启 docker 守护进程**
   算力机上可能跑着其他重要容器（我们的 way 上有 2 个 vLLM 服务）。重启 docker 会杀掉它们。
   本方案全程只需 `docker load` / `docker run`，不触碰守护进程配置。

## 三点五、启动参数（渲染服务）

镜像默认 CMD 即启动渲染服务：

```bash
media-factory render-server --port 7788 --home /data --token-file /data/render-token
```

| 参数 | 说明 |
|---|---|
| `--token-file` | token 文件（内容即 token）；文件不存在则不校验（会有提示） |
| `--max-concurrent` | 并发渲染上限（默认 1；32 线程可到 2） |
| `--hyperframes-bin` / `--gsap-js` | 覆盖容器内默认路径（一般不用改） |

云端配置（`~/.media-factory/config.yaml`）：

```yaml
video:
  mode: dynamic                 # dynamic | cover
  dynamic:
    renderer_url: http://<算力机 tailscale IP>:7788
    callback_base: http://<云端公网地址>:8092   # 算力机据此下载素材与回传成品
    token: <与容器内 render-token 一致>
    fps: 24
    quality: looks
    workers: 16
    on_unavailable: cover       # 算力机不可用时：cover（先出静态版）| queue（排队等待）
    timeout_seconds: 900
```

## 四、验证

```bash
# 容器状态
sudo docker ps --filter name=mf-render

# 环境自检（Node / ffmpeg / 中文字体 / hyperframes）
sudo docker exec mf-render bash -c \
  'node -v; ffmpeg -version | head -1; fc-list | grep -c CJK; /opt/mf-render/node_modules/.bin/hyperframes --version'

# 渲染冒烟（20 秒样片）
sudo docker exec mf-render bash -c \
  'cd /data/demo && /opt/mf-render/node_modules/.bin/hyperframes render -o renders/smoke.mp4 -q looks -f 24 -w 16'
```

实测（way：Ryzen AI MAX+ 395 / 32 线程）：20 秒 1080p 视频**约 7.3 秒**渲染完成，
外推 4.4 分钟音频约 1.5~2 分钟。容器空闲占用约 7MB 内存。

## 五、升级

```bash
# 在构建机构建新版本 → 传输 → 重建容器（数据卷 /data 不变）
docker build --platform linux/amd64 -t mf-render:0.2.4 .
docker save mf-render:0.2.4 | gzip -1 | ssh <算力机> 'gunzip | sudo docker load'
ssh <算力机> 'sudo docker rm -f mf-render && sudo docker run -d --name mf-render ... mf-render:0.2.4 ...'
```

## 六、清理

```bash
ssh <算力机> 'sudo docker rm -f mf-render && sudo docker rmi mf-render:0.3.1'
```
容器删除后宿主完全恢复原状（无任何 Node/Chromium/字体残留）。
