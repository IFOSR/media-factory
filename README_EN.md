# Media Factory · Content Factory for Creators

English | [中文](./README.md)

Turn a **reference article** into a ready-to-publish content pack: viral rewrite + AI cover image + two-host podcast audio + final video (with subtitles) — all from a single command.

```
Reference ──► ① Rewrite (viral copy) ──► ② Image (cover) ──► ③ Podcast (2 hosts) ──► ④ Video (image + audio + subs)
```

## Core Features

- **4-step automated pipeline** — Article in, `copy + image + podcast + video` out. Run everything at once, or execute any single step and resume after failures
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

## Quick Start (3 steps)

```bash
# ① Configure: interactive wizard (or the ⚙ panel in the web UI)
media-factory config

# ② Start the web app
media-factory serve            # open http://localhost:8092

# ③ In the web UI: + New Task → paste reference text → 🚀 Run all
```

Or pure CLI:

```bash
media-factory run input.md --disclaimer --size portrait
# Step by step: rewrite / image / podcast / video — artifacts in output/<task-id>/
```

<details>
<summary>Web UI highlights</summary>

- **Sidebar**: task list (status dot + auto-extracted title); click to replay any task's full timeline; delete single or clear all
- **Step cards, three states**: input (optional per-step settings) → thinking (streaming logs) → done (inline artifacts: editable text, zoomable image, playable audio/video)
- **Re-run**: per-step "↻ Re-run" prefills previous inputs; top-bar "↻ Re-run all" reruns the pipeline; artifacts refresh in place
- **Image options**: prompt + 📎 multiple reference images + size + disclaimer checkbox — all per-task

</details>

<details>
<summary>Full CLI reference</summary>

```bash
media-factory run <input> [--id ID] [--ref IMG]... [--prompt S] [--image-prompt S]
                  [--podcast-prompt S] [--disclaimer] [--size square|portrait|landscape]
media-factory rewrite <input> [--prompt S]
media-factory image   [--id ID] [--ref IMG]... [--prompt S] [--disclaimer] [--size ...]
media-factory podcast [--id ID] [--script] [--prompt S]
media-factory video   [--id ID]
media-factory config      # interactive wizard
media-factory serve       # web server (--port 8092)
```

Resume after failure: `media-factory podcast --id <task-id>` (upstream artifacts are on disk).

</details>

## Configuration (`~/.media-factory/config.yaml`)

- **LLM**: default `pi` (authenticate with `pi auth login`); or any OpenAI-compatible provider (e.g. Deepseek: BaseURL + API key + model)
- **Image**: `nano-banana` (official Gemini, default) / `openai-image` (gpt-image) / custom OpenAI-compatible
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
  └── video.mp4                 # final video
```

## Platform Support

| Platform | Prebuilt |
|----------|----------|
| macOS Apple Silicon / Intel | ✅ |
| Linux x64 (incl. WSL; needs ffmpeg + fonts-noto-cjk) | ✅ |
| Windows x64 (`media-factory.exe`; serve.sh not applicable) | ✅ |

## Development

```bash
cargo test    # 41 tests (protocol wiremock / real ffmpeg muxing / e2e pipeline)
cargo clippy  # 0 warnings
./serve.sh restart   # local web service management
```

Design docs in `docs/plans/`.

## License

MIT
