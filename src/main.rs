use clap::{Parser, Subcommand};

mod config;
mod llm;
mod pi_rpc;

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
    },
    /// 步骤 2：基于改写文案生成配图
    Image {
        #[arg(long)] id: Option<String>,
        /// 可选参考图
        #[arg(long)] r#ref: Option<String>,
    },
    /// 步骤 3：基于改写文案生成播客
    Podcast {
        #[arg(long)] id: Option<String>,
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
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        _ => println!("not implemented"),
    }
    Ok(())
}
