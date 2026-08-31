use clap::{Parser, Subcommand};

mod cmd;
mod config;
mod ffmpeg;
mod llm;
mod pi_rpc;
mod podcast;
mod provider;
mod server;
mod tts;
mod wizard;

#[derive(Parser)]
#[command(name = "media-factory", about = "自媒体内容工厂：改写 → 生图 → 播客 → 视频")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 交互式配置向导
    Config,
    /// 步骤 1：改写参考文案
    Rewrite {
        /// 参考文案文件路径（缺省读 stdin）
        input: Option<String>,
        /// 任务 id（缺省新建）
        #[arg(long)] id: Option<String>,
        /// 用户自定义改写要求（叠加在系统爆款要求之上）
        #[arg(long)] prompt: Option<String>,
    },
    /// 步骤 2：基于改写文案生成配图
    Image {
        #[arg(long)] id: Option<String>,
        /// 可选参考图
        #[arg(long)] r#ref: Option<String>,
        /// 用户自定义生图要求（叠加在系统提炼要求之上）
        #[arg(long)] prompt: Option<String>,
    },
    /// 步骤 3：基于改写文案生成播客
    Podcast {
        #[arg(long)] id: Option<String>,
        /// 模式 B：先生成脚本（脚本已存在时直接按脚本合成）
        #[arg(long)] script: bool,
        /// 用户自定义播客风格要求
        #[arg(long)] prompt: Option<String>,
    },
    /// 步骤 4：图片 + 播客合成视频
    Video {
        #[arg(long)] id: Option<String>,
    },
    /// 串联执行全部四步
    Run {
        input: Option<String>,
        #[arg(long)] id: Option<String>,
        #[arg(long)] r#ref: Option<String>,
        /// 用户自定义改写要求
        #[arg(long)] prompt: Option<String>,
        /// 用户自定义生图要求
        #[arg(long)] image_prompt: Option<String>,
        /// 用户自定义播客风格要求
        #[arg(long)] podcast_prompt: Option<String>,
    },
    /// 启动 Web 服务（功能与 CLI 一致）
    Serve {
        /// 监听端口
        #[arg(long, default_value_t = 8092)]
        port: u16,
    },
}

fn require_pi() -> anyhow::Result<()> {
    let ok = std::process::Command::new("pi")
        .arg("--version")
        .output()
        .is_ok();
    anyhow::ensure!(ok, "未找到 pi，请先安装（npm install -g @earendil-works/pi-coding-agent）");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // 前置检查：除 config 外都需要 pi；video/run 需要 ffmpeg
    if !matches!(&cli.command, Commands::Config) {
        require_pi()?;
    }
    if matches!(&cli.command, Commands::Video { .. } | Commands::Run { .. }) {
        ffmpeg::require_ffmpeg()?;
    }

    match cli.command {
        Commands::Config => wizard::run().await?,
        Commands::Rewrite { input, id, prompt } => {
            cmd::rewrite::run(input, id, prompt).await?;
        }
        Commands::Image { id, r#ref, prompt } => {
            cmd::image::run(id, r#ref, prompt).await?;
        }
        Commands::Podcast { id, script, prompt } => {
            cmd::podcast::run(id, script, prompt).await?;
        }
        Commands::Video { id } => {
            cmd::video::run(id)?;
        }
        Commands::Run { input, id, r#ref, prompt, image_prompt, podcast_prompt } => {
            cmd::run::run(input, id, r#ref, prompt, image_prompt, podcast_prompt).await?;
        }
        Commands::Serve { port } => {
            server::run(port).await?;
        }
    }
    Ok(())
}
