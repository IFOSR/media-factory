//! 火山播客大模型客户端（播客 API WebSocket v3）。
//! 文档：https://www.volcengine.com/docs/6561/1668014

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::volc_proto::{self, Frame};

pub struct PodcastRequest {
    /// action=0：喂文本，模型自动生成双人播客
    pub input_text: Option<String>,
    /// action=3：按给定轮次文本合成
    pub nlp_texts: Option<Vec<serde_json::Value>>,
    /// 只输出脚本文本，不合成音频（action=0 下生效）
    pub only_nlp_text: bool,
}

pub enum PodcastResult {
    /// 完整播客音频（mp3 字节）
    Audio(Vec<u8>),
    /// 播客脚本文本（only_nlp_text 模式）
    ScriptText(String),
}

pub struct VolcPodcast {
    appid: String,
    access_token: String,
    speakers: (String, String),
    ws_url: String,
    client: reqwest::Client,
}

impl VolcPodcast {
    pub fn new(appid: String, access_token: String, speakers: (String, String)) -> Self {
        Self {
            appid,
            access_token,
            speakers,
            ws_url: "wss://openspeech.bytedance.com/api/v3/sami/podcasttts".into(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    /// 测试用：覆盖 ws_url
    pub fn with_ws_url(mut self, url: String) -> Self {
        self.ws_url = url;
        self
    }

    /// 当前配置的两个发音人 (host, guest)
    pub fn speakers(&self) -> (String, String) {
        self.speakers.clone()
    }

    fn session_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
    }

    fn build_payload(&self, req: &PodcastRequest) -> serde_json::Value {
        let input_id = uuid::Uuid::new_v4().to_string();
        if let Some(text) = &req.input_text {
            serde_json::json!({
                "input_id": input_id,
                "input_text": text,
                "action": 0,
                "use_head_music": false,
                "use_tail_music": false,
                "audio_config": {"format": "mp3", "sample_rate": 24000},
                "input_info": {
                    "return_audio_url": true,
                    "only_nlp_text": req.only_nlp_text,
                    "input_text_max_length": 12000
                }
            })
        } else {
            serde_json::json!({
                "input_id": input_id,
                "action": 3,
                "nlp_texts": req.nlp_texts,
                "use_head_music": false,
                "use_tail_music": false,
                "speaker_info": {"speakers": [self.speakers.0, self.speakers.1]},
                "audio_config": {"format": "mp3", "sample_rate": 24000},
                "input_info": {"return_audio_url": true}
            })
        }
    }

    pub async fn generate(&self, req: &PodcastRequest) -> anyhow::Result<PodcastResult> {
        let session_id = Self::session_id();

        // 建连（带鉴权 headers）
        let mut request = self.ws_url.clone().into_client_request()?;
        let h = request.headers_mut();
        h.insert("X-Api-App-Id", self.appid.parse().unwrap());
        h.insert("X-Api-Access-Key", self.access_token.parse().unwrap());
        h.insert(
            "X-Api-Resource-Id",
            "volc.service_type.10050".parse().unwrap(),
        );
        h.insert("X-Api-App-Key", "aGjiRDfUWi".parse().unwrap());
        h.insert(
            "X-Api-Request-Id",
            uuid::Uuid::new_v4().to_string().parse().unwrap(),
        );

        let (mut ws, _resp) = tokio_tungstenite::connect_async(request).await?;

        // 发送 StartSession
        let payload = self.build_payload(req);
        let frame = volc_proto::encode_client_frame(
            volc_proto::EV_START_SESSION,
            &session_id,
            payload.to_string().as_bytes(),
        );
        ws.send(Message::Binary(frame)).await?;

        let mut audio_buf: Vec<u8> = Vec::new();
        let mut audio_url: Option<String> = None;
        let mut script_lines: Vec<String> = Vec::new();

        while let Some(msg) = ws.next().await {
            let msg = msg?;
            if let Message::Binary(data) = msg {
                let f: Frame = volc_proto::decode_frame(&data)?;
                if f.message_type == volc_proto::MSG_ERROR {
                    anyhow::bail!(
                        "播客 API 返回错误（code {}）: {}",
                        f.event,
                        String::from_utf8_lossy(&f.payload)
                    );
                }
                match f.event {
                    volc_proto::EV_PODCAST_ROUND_RESPONSE => {
                        audio_buf.extend_from_slice(&f.payload);
                    }
                    volc_proto::EV_PODCAST_ROUND_START => {
                        if req.only_nlp_text {
                            let v: serde_json::Value = serde_json::from_slice(&f.payload)?;
                            let speaker = v["speaker"].as_str().unwrap_or("");
                            let text = v["text"].as_str().unwrap_or("");
                            script_lines.push(format!("{speaker}：{text}"));
                        }
                    }
                    volc_proto::EV_PODCAST_END => {
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&f.payload) {
                            audio_url = v["meta_info"]["audio_url"].as_str().map(|s| s.to_string());
                        }
                    }
                    volc_proto::EV_SESSION_FINISHED => break,
                    _ => {}
                }
            } else if msg.is_close() {
                break;
            }
        }

        // 结束连接
        let finish = volc_proto::encode_client_frame(volc_proto::EV_FINISH_CONNECTION, &session_id, b"");
        let _ = ws.send(Message::Binary(finish)).await;
        let _ = ws.close(None).await;

        if req.only_nlp_text {
            anyhow::ensure!(
                !script_lines.is_empty(),
                "播客 API 未返回脚本文本"
            );
            return Ok(PodcastResult::ScriptText(script_lines.join("\n")));
        }

        if let Some(url) = audio_url {
            let resp = self.client.get(&url).send().await?;
            anyhow::ensure!(resp.status().is_success(), "下载播客音频失败: {}", resp.status());
            return Ok(PodcastResult::Audio(resp.bytes().await?.to_vec()));
        }

        anyhow::ensure!(!audio_buf.is_empty(), "播客 API 未返回音频");
        Ok(PodcastResult::Audio(audio_buf))
    }
}
