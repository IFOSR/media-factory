# Media Factory · Content Factory for Creators

English | [中文](./README.md)

Turn a **reference article** into a ready-to-publish content pack: viral rewrite + AI cover image + two-host podcast audio + final video (with subtitles) — all from a single command.

```
Reference ──► ① Rewrite ──► ② Image ──► ③ Podcast (2 hosts) ──► ④ Scenes (visual script) ──► ⑤ Video
```

Two video modes: **dynamic explainer visuals** synced to the audio (needs one compute machine), or **static cover video** (fastest, single machine).

## Core Features

- **5-step automated pipeline** — Article in, `copy + image + podcast + scenes + video` out. Run everything at once, or execute any single step and resume after failures
- **Dynamic explainer visuals** — Auto-generate visuals synced **segment-by-segment** with the audio (bullet cards / metric cards / bar charts / quotes / chapter slides). Aspect ratio follows the cover image (9:16 / 16:9 / 1:1), with a dedicated portrait layout; switch back to a static cover video anytime
- **Agent thinking-chain visualization** — The web UI streams what the backend is doing at every step (reading input → calling model → producing artifact), as transparent as watching an AI agent work
- **Fully editable intermediates** — Every artifact (rewritten copy / podcast script) is previewable, **editable and saveable**; downstream steps automatically use your edits. Any step can be **re-run** with previous inputs prefilled, artifacts refresh in place, and downstream steps get an "upstream updated" hint
- **Pluggable providers** — LLM (built-in pi / any OpenAI-compatible such as Deepseek), image (Gemini / OpenAI-compatible), podcast (Volcano "Podcast TTS" model) — mix and match via wizard or web panel
- **Publishing-ready details** — Image sizes (1:1 / portrait 9:16 / landscape 16:9), multiple reference images, **auto disclaimer overlay** (compliance for finance content), word-boundary-safe English subtitles, automatic host/guest role detection, automatic task titles
- **CLI + Web dual mode** — One command in the terminal; or the web app (default `http://localhost:8092`) with sidebar task management, step cards, inline playback, dark/light themes
- **Cross-platform** — Prebuilt binaries for macOS (Apple Silicon/Intel), Linux, and Windows; one-command install

## Quick Install

### One command (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/IFOSR/media-factory/main/install.sh | bash
```

Download sources: **self-hosted mirror first (China-friendly, md5-verified) → GitHub fallback → source-build fallback** (installs rustup if Rust is missing).

```bash
./install.sh --mirror   # force self-hosted mirror
./install.sh --github   # force GitHub
MF_MIRROR=https://your-mirror ./install.sh   # override mirror URL
```

<details>
<summary>More options</summary>

```bash
git clone https://github.com/IFOSR/media-factory.git && cd media-factory
./install.sh                # Release first, source fallback
./install.sh --release      # prebuilt only
./install.sh --source       # build from source only
./install.sh --bin-dir /usr/local/bin   # custom dir (default ~/.media-factory/bin)
```

Manual: install [Rust](https://rustup.rs), then `cargo build --release`.

</details>

### Runtime dependencies

| Dependency | Required for | Install |
|------------|--------------|---------|
| ffmpeg | podcast / video steps | `brew install ffmpeg` / `apt install ffmpeg` / `winget install ffmpeg` |
| pi | default LLM (replaceable with custom providers) | `npm install -g @earendil-works/pi-coding-agent` |

> Dynamic explainer visuals additionally need one compute machine (Docker-deployed render worker, see below). Neither end users nor the server need Node/Chromium installed.

## Quick Start (3 steps)

```bash
# ① Configure: interactive wizard (or the ⚙ panel in the web UI)
media-factory config

# ② Start the web app
media-factory serve            # start in background, open http://localhost:8092 (on a server use http://<IP>:8092)

# ③ In the web UI: + New Task → paste reference text → 🚀 Run all
```

Or pure CLI:

```bash
media-factory run input.md --disclaimer --size portrait
# Step by step: rewrite / image / podcast / scenes / video — artifacts in output/<task-id>/
```

<details>
<summary>Web UI highlights</summary>

- **Sidebar**: task list (status dot + auto-extracted title); click to replay any task's full timeline; delete single or clear all
- **Step cards, three states**: input (optional per-step settings) → thinking (streaming logs) → done (inline artifacts: editable text, zoomable image, playable audio/video)
- **Re-run**: per-step "↻ Re-run" prefills previous inputs; top-bar "↻ Re-run all" reruns the pipeline; artifacts refresh in place
- **Image options**: prompt + 📎 multiple reference images + size + disclaimer checkbox — all per-task
- **Scenes card**: pick the visual mode (dynamic explainer / static cover); the generated `scene.json` is editable — re-run the video step after edits
- **Render progress**: in dynamic mode the compute machine's live progress (download → render % → mux → upload) streams into the card

</details>

<details>
<summary>Full CLI reference</summary>

```bash
media-factory run <input> [--id ID] [--ref IMG]... [--prompt S] [--image-prompt S]
                  [--podcast-prompt S] [--disclaimer] [--size square|portrait|landscape]
media-factory rewrite <input> [--prompt S]
media-factory image   [--id ID] [--ref IMG]... [--prompt S] [--disclaimer] [--size ...]
media-factory podcast [--id ID] [--script] [--prompt S]
media-factory scenes  [--id ID]                 # build scene.json (dynamic visuals)
media-factory video   [--id ID]                 # compose video (dynamic / cover per config)
media-factory config      # interactive wizard
media-factory serve       # web server (background; --port sets port, --foreground for debugging)
media-factory render-server --port 7788 --home /data   # render service for the compute machine
```

Resume after failure: `media-factory podcast --id <task-id>` (upstream artifacts are on disk).

</details>

## Dynamic explainer visuals (optional)

By default the video is "cover image + audio + subtitles". With dynamic mode enabled, the pipeline gains a **Scenes** step that produces visuals synced **segment-by-segment** with the audio.

```
Rewrite → Image → Podcast → Scenes → Video
                              │        │
                        scene.json     dynamic visuals + audio + subtitles
                        (editable)
```

**Quality guarantees**

| Mechanism | Description |
|---|---|
| Zero timeline drift | Scene boundaries snap to subtitle timestamps (never model-estimated) |
| No stale visuals | Scenes must cover **every** subtitle segment; uncovered spans get a synthesized quote card from that exact transcript |
| No invented numbers | Metric/chart numbers must exist in the transcript (Chinese numerals supported), otherwise the scene degrades to a bullet card |
| Aspect follows the cover | 9:16 → 1080×1920, 16:9 → 1920×1080, 1:1 → 1080×1080; portrait gets a dedicated layout |
| Never breaks delivery | If the render machine is unavailable or times out → automatic fallback to static cover video |

Scene types: cover / chapter / bullets / metric (count-up) / bar chart / quote / end.

### Enabling

- **Web UI**: on the **Scenes** card pick "dynamic explainer"
- **Default**: ⚙ config panel → "Video"; or edit the config file (below)

### Compute machine (render worker)

Dynamic rendering needs CPU (Chromium renders frame by frame). It runs in a **Docker container** and leaves the host untouched:

```bash
# 1) Build the image on a machine with Docker Hub access (see scripts/render-worker/Dockerfile)
cd scripts/render-worker && docker build --platform linux/amd64 -t mf-render:0.3.4 .

# 2) Ship it to the compute machine and start (~2.7GB, ~1 min over LAN)
docker save mf-render:0.3.4 | gzip -1 | ssh <host> 'gunzip | sudo docker load'
ssh <host> 'sudo docker run -d --name mf-render --restart unless-stopped \
  -p 7788:7788 -v /srv/mf-render:/data \
  -e HTTP_PROXY= -e HTTPS_PROXY= mf-render:0.3.4'
```

Server config (`~/.media-factory/config.yaml`):

```yaml
video:
  mode: dynamic                 # dynamic | cover
  dynamic:
    renderer_url: http://<compute-host>:7788
    callback_base: http://<this-server-public-url>:8092   # worker downloads media & uploads results here
    token: <must match /data/render-token on the worker>
    fps: 24                     # 24 | 30
    quality: looks              # draft | looks | delivery
    on_unavailable: cover       # cover = deliver static video | queue = wait for the worker
```

> After a successful upload the worker **deletes its local video and intermediates** (only the server keeps them).
> Measured: a 4.4-minute audio renders in ~1–1.5 minutes on a 32-thread machine.
> Full deployment/upgrade/self-check guide: [`scripts/render-worker/README.md`](scripts/render-worker/README.md).

## Configuration (`~/.media-factory/config.yaml`)

- **LLM**: default `pi` (authenticate with `pi auth login`); or any OpenAI-compatible provider (e.g. Deepseek: BaseURL + API key + model)
- **Image**: `nano-banana` (official Gemini, default) / `openai-image` (gpt-image) / `doubao-seedream` (Doubao Seedream 4.0 via Ark: multi-image reference, true 9:16/16:9, watermark off; model name or ep- endpoint) / custom OpenAI-compatible
- **Podcast**: `volc-podcast` (Volcano Podcast TTS, recommended; needs Access Token + appid from the [console](https://console.volcengine.com/speech/service/10028)) / generic TTS (openai-tts etc.: script → per-turn synthesis → concat)
  - The Volcano model is a **two-host dialogue** model (random opening speaker, roles auto-detected)

Podcast modes: A (default) synthesize directly from text; B (`--script`) generate an editable script first, then re-run to synthesize.

## Artifacts

```
output/<task-id>/
  ├── input.md / rewritten.md   # reference / rewritten copy (editable; edits feed downstream)
  ├── image.png                 # cover image
  ├── script.md                 # podcast script (mode B / TTS)
  ├── podcast.mp3 / subtitle.srt# audio / subtitles
  ├── scene.json                # scene script (dynamic mode, editable → re-render)
  └── video.mp4                 # final video (dynamic visuals or static cover)
```

## Platform Support

| Platform | Prebuilt |
|----------|----------|
| macOS Apple Silicon / Intel | ✅ |
| Linux x64 (incl. WSL; needs ffmpeg + fonts-noto-cjk) | ✅ |
| Windows x64 (`media-factory.exe`; background serve not applicable, use `--foreground`) | ✅ |

## Development

```bash
cargo test    # 67 tests (protocol wiremock / real ffmpeg muxing / scene coverage / e2e pipeline)
cargo clippy  # 0 warnings
media-factory serve --restart   # manage web service: --stop / --restart / --status
```

Design docs in `docs/plans/`.

## License

MIT
