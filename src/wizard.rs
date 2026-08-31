use std::collections::HashMap;

use dialoguer::{Confirm, Input, Password, Select};

use crate::config::{BuiltinKind, Config, ProviderConfig, TaskKind, TaskSelection};
use crate::pi_rpc;

/// 该媒体任务可用的内置 provider 列表
pub fn builtin_for_task(task: TaskKind) -> Vec<BuiltinKind> {
    [
        BuiltinKind::NanoBanana,
        BuiltinKind::OpenAiImage,
        BuiltinKind::DoubaoSeedream,
        BuiltinKind::VolcPodcast,
        BuiltinKind::GeminiTts,
        BuiltinKind::OpenAiTts,
        BuiltinKind::VolcTts,
    ]
    .into_iter()
    .filter(|k| k.supports(task))
    .collect()
}

pub fn validate_custom_name(
    existing: &HashMap<String, ProviderConfig>,
    name: &str,
) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("名称不能为空");
    }
    if existing.contains_key(name) {
        anyhow::bail!("provider 名称已存在: {name}");
    }
    Ok(())
}

pub fn validate_base_url(url: &str) -> anyhow::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        anyhow::bail!("BaseURL 必须以 http:// 或 https:// 开头");
    }
    Ok(())
}

/// 把 pi get_available_models 响应解析为 "provider/id" 列表
pub fn parse_available_models(resp: &serde_json::Value) -> Vec<String> {
    resp["models"]
        .as_array()
        .map(|ms| {
            ms.iter()
                .filter_map(|m| {
                    Some(format!(
                        "{}/{}",
                        m["provider"].as_str()?,
                        m["id"].as_str()?
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn builtin_display_name(kind: BuiltinKind) -> &'static str {
    match kind {
        BuiltinKind::NanoBanana => "nano-banana（官方 Gemini 生图，支持参考图）",
        BuiltinKind::OpenAiImage => "openai-image（gpt-image）",
        BuiltinKind::DoubaoSeedream => "doubao-seedream（豆包生图）",
        BuiltinKind::VolcPodcast => "volc-podcast（火山播客大模型，推荐）",
        BuiltinKind::GeminiTts => "gemini-tts（通用 TTS）",
        BuiltinKind::OpenAiTts => "openai-tts（通用 TTS）",
        BuiltinKind::VolcTts => "volc-tts（火山豆包语音）",
        BuiltinKind::Pi => "pi（pi agent 语言模型，默认）",
    }
}

/// 内置 provider 在 config.yaml 中的稳定 ID
fn builtin_id(kind: BuiltinKind) -> &'static str {
    match kind {
        BuiltinKind::NanoBanana => "nano-banana",
        BuiltinKind::OpenAiImage => "openai-image",
        BuiltinKind::DoubaoSeedream => "doubao-seedream",
        BuiltinKind::VolcPodcast => "volc-podcast",
        BuiltinKind::GeminiTts => "gemini-tts",
        BuiltinKind::OpenAiTts => "openai-tts",
        BuiltinKind::VolcTts => "volc-tts",
        BuiltinKind::Pi => "pi",
    }
}

fn builtin_extra_keys(kind: BuiltinKind) -> &'static [&'static str] {
    match kind {
        BuiltinKind::VolcPodcast => &["appid"],
        BuiltinKind::VolcTts => &["appid", "cluster"],
        _ => &[],
    }
}

pub async fn run() -> anyhow::Result<()> {
    let path = Config::path();
    let mut cfg = Config::load(&path)?;
    loop {
        let items = [
            "配置「语言模型」（pi agent）",
            "配置「生图」provider",
            "配置「播客」provider",
            "新增自定义 provider（Other，仅限生图/播客）",
            "保存并退出",
            "不保存退出",
        ];
        let sel = Select::new()
            .with_prompt("Media Factory 配置")
            .items(&items)
            .default(0)
            .interact()?;
        match sel {
            0 => bind_llm(&mut cfg).await?,
            1 => bind_media(&mut cfg, TaskKind::Image)?,
            2 => bind_media(&mut cfg, TaskKind::Podcast)?,
            3 => {
                let name = add_custom(&mut cfg)?;
                println!("已创建自定义 provider「{name}」，可在「生图」/「播客」中绑定");
            }
            4 => {
                cfg.save(&path)?;
                println!("已保存到 {}", path.display());
                return Ok(());
            }
            _ => {
                if Confirm::new()
                    .with_prompt("确定放弃修改？")
                    .interact()?
                {
                    return Ok(());
                }
            }
        }
    }
}

async fn bind_llm(cfg: &mut Config) -> anyhow::Result<()> {
    let resp = pi_rpc::rpc_once(
        std::path::Path::new("pi"),
        None,
        serde_json::json!({"type": "get_available_models"}),
    )
    .await?;

    let models = parse_available_models(&resp);
    if models.is_empty() {
        println!("⚠️  pi 当前没有已认证可用的模型。");
        println!("   请先在终端执行 `pi auth login` 配置 provider 认证，");
        println!("   或参考 pi 的 models.json 添加自定义 provider。");
        return Ok(());
    }

    let mut items: Vec<String> = vec!["← 返回上一步".into(), "使用 pi 默认模型".into()];
    items.extend(models.iter().cloned());

    let sel = Select::new()
        .with_prompt("选择语言模型")
        .items(&items)
        .default(0)
        .interact()?;

    if sel == 0 {
        return Ok(());
    }
    cfg.tasks.llm = if sel == 1 {
        None
    } else {
        // 把选择的 pi 模型写入内置 pi provider 的 extra.model
        let model = models[sel - 2].clone();
        let mut extra = std::collections::HashMap::new();
        extra.insert("model".to_string(), model);
        cfg.providers.insert(
            "pi".to_string(),
            ProviderConfig::Builtin {
                kind: BuiltinKind::Pi,
                api_key: String::new(),
                extra,
            },
        );
        Some(crate::config::LlmSelection::Provider(TaskSelection {
            provider: "pi".to_string(),
        }))
    };
    Ok(())
}

fn bind_media(cfg: &mut Config, task: TaskKind) -> anyhow::Result<()> {
    let builtins = builtin_for_task(task);

    // 菜单：返回上一步 + 内置 provider + 已有自定义 + Other（新增）
    let mut labels: Vec<String> = vec!["← 返回上一步".into()];
    labels.extend(builtins.iter().map(|k| {
        let configured = cfg
            .providers
            .iter()
            .any(|(_, p)| matches!(p, ProviderConfig::Builtin { kind, .. } if kind == k));
        let mark = if configured { " ✓" } else { " [未配置]" };
        format!("{}{}", builtin_display_name(*k), mark)
    }));

    let custom_names: Vec<String> = cfg
        .providers
        .iter()
        .filter(|(_, p)| matches!(p, ProviderConfig::Custom { .. }))
        .map(|(n, _)| n.clone())
        .collect();
    labels.extend(custom_names.iter().map(|n| format!("{n}（自定义）")));
    let other_idx = labels.len();
    labels.push("Other（新增自定义 provider）".into());

    let sel = Select::new()
        .with_prompt(match task {
            TaskKind::Image => "选择生图 provider",
            TaskKind::Podcast => "选择播客 provider",
            TaskKind::Llm => "选择语言模型 provider",
        })
        .items(&labels)
        .default(0)
        .interact()?;

    // 0 = 返回；other_idx = Other；1..=builtins.len() = 内置；其余 = 已有自定义
    if sel == 0 {
        return Ok(());
    }
    if sel == other_idx {
        let name = add_custom(cfg)?;
        let provider = TaskSelection { provider: name };
        match task {
            TaskKind::Image => cfg.tasks.image = Some(provider),
            TaskKind::Podcast => cfg.tasks.podcast = Some(provider),
            TaskKind::Llm => cfg.tasks.llm = Some(crate::config::LlmSelection::Provider(provider)),
        }
        return Ok(());
    }
    if sel <= builtins.len() {
        let kind = builtins[sel - 1];
        let id = builtin_id(kind).to_string();
        // 若未配置 api_key，录入
        let has_key = cfg.providers.iter().any(|(_, p)| {
            matches!(p, ProviderConfig::Builtin { kind: k, api_key, .. } if k == &kind && !api_key.is_empty())
        });
        if !has_key {
            let key = Password::new()
                .with_prompt(format!("输入 {} 的 API Key / Access Token", builtin_display_name(kind)))
                .interact()?;
            let mut extra = HashMap::new();
            for k in builtin_extra_keys(kind) {
                let v = Input::<String>::new()
                    .with_prompt(format!("输入 {k}"))
                    .interact()?;
                extra.insert(k.to_string(), v);
            }
            cfg.providers.insert(
                id.clone(),
                ProviderConfig::Builtin {
                    kind,
                    api_key: key,
                    extra,
                },
            );
        }
        let selection = TaskSelection { provider: id };
        match task {
            TaskKind::Image => cfg.tasks.image = Some(selection),
            TaskKind::Podcast => cfg.tasks.podcast = Some(selection),
            TaskKind::Llm => cfg.tasks.llm = Some(crate::config::LlmSelection::Provider(selection)),
        }
    } else {
        let name = custom_names[sel - 1 - builtins.len()].clone();
        let provider = TaskSelection { provider: name };
        match task {
            TaskKind::Image => cfg.tasks.image = Some(provider),
            TaskKind::Podcast => cfg.tasks.podcast = Some(provider),
            TaskKind::Llm => cfg.tasks.llm = Some(crate::config::LlmSelection::Provider(provider)),
        }
    }
    Ok(())
}

fn add_custom(cfg: &mut Config) -> anyhow::Result<String> {
    let name = Input::<String>::new()
        .with_prompt("自定义 provider 名称")
        .interact()?;
    validate_custom_name(&cfg.providers, &name)?;

    let base_url = Input::<String>::new()
        .with_prompt("BaseURL")
        .interact()?;
    validate_base_url(&base_url)?;

    let api_key = Password::new()
        .with_prompt("API Key")
        .interact()?;

    let model = Input::<String>::new()
        .with_prompt("模型名")
        .interact()?;

    cfg.providers.insert(
        name.clone(),
        ProviderConfig::Custom {
            base_url,
            api_key,
            model,
        },
    );
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lists_match_tasks() {
        assert_eq!(builtin_for_task(TaskKind::Image).len(), 3);
        assert_eq!(builtin_for_task(TaskKind::Podcast).len(), 4);
        assert!(builtin_for_task(TaskKind::Podcast).contains(&BuiltinKind::VolcPodcast));
        assert!(!builtin_for_task(TaskKind::Image).contains(&BuiltinKind::VolcPodcast));
    }

    #[test]
    fn custom_name_and_url_validation() {
        let mut m = HashMap::new();
        assert!(validate_custom_name(&m, "").is_err());
        assert!(validate_custom_name(&m, "x").is_ok());
        m.insert(
            "x".into(),
            ProviderConfig::Builtin {
                kind: BuiltinKind::GeminiTts,
                api_key: "k".into(),
                extra: Default::default(),
            },
        );
        assert!(validate_custom_name(&m, "x").is_err());

        assert!(validate_base_url("https://api.x.com/v1").is_ok());
        assert!(validate_base_url("api.x.com").is_err());
    }

    #[test]
    fn parses_pi_model_list() {
        let v = serde_json::json!({"type":"response","success":true,"data":{"models":[
            {"provider":"google","id":"gemini-2.5-pro","name":"Gemini 2.5 Pro"},
            {"provider":"openai","id":"gpt-5","name":"GPT-5"}]}});
        assert_eq!(
            parse_available_models(&v["data"]),
            vec!["google/gemini-2.5-pro", "openai/gpt-5"]
        );
    }
}
