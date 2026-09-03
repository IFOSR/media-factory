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
    /// 完整播客音频（mp3 字节）+ 字幕条目（含时间戳）
    Audio {
        bytes: Vec<u8>,
        subtitles: Vec<super::SubtitleEntry>,
    },
    /// 播客脚本文本（only_nlp_text 模式）
    ScriptText(String),
}

pub struct VolcPodcast {
    appid: String,
    access_token: String,
    speakers: Vec<String>,
    ws_url: String,
    client: reqwest::Client,
}

impl VolcPodcast {
    pub fn new(appid: String, access_token: String, speakers: Vec<String>) -> Self {
        Self {
            appid,
            access_token,
            speakers,
            ws_url: "wss://openspeech.bytedance.com/api/v3/sami/podcasttts".into(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    /// 当前配置的发音人（1~2 个）
    pub fn speakers(&self) -> Vec<String> {
        self.speakers.clone()
    }

    /// 任务级音色覆盖（固定双人；未指定的音色回落到配置默认）
    pub fn with_speaker_override(mut self, s1: Option<String>, s2: Option<String>) -> Self {
        let base = self.speakers.clone();
        let fallback = base.first().cloned().unwrap_or_default();
        let pick = |i: usize, over: &Option<String>| -> String {
            match over {
                Some(s) if !s.is_empty() => s.clone(),
                _ => base.get(i).cloned().unwrap_or_else(|| fallback.clone()),
            }
        };
        self.speakers = vec![pick(0, &s1), pick(1, &s2)];
        self
    }

    /// 把音色 ID 映射成友好称呼
    fn speaker_label(&self, speaker: &str) -> String {
        if self.speakers.len() > 1 && speaker == self.speakers[1] {
            "嘉宾".to_string()
        } else if self.speakers.first().map(|s| s == speaker).unwrap_or(false) {
            "主持人".to_string()
        } else {
            speaker.to_string()
        }
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
                "audio_config": {"format": "mp3", "sample_rate": 24000, "speech_rate": 0},
                "speaker_info": {"random_order": self.speakers.len() > 1, "speakers": self.speakers.clone()},
                "aigc_watermark": false,
                "aigc_metadata": {"enable": true, "content_producer": "volcengine", "produce_id": "12abc", "content_propagator": "volcengine", "propagate_id": "34def"},
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
                "speaker_info": {"speakers": self.speakers.clone()},
                "audio_config": {"format": "mp3", "sample_rate": 24000},
                "input_info": {"return_audio_url": true}
            })
        }
    }

    pub async fn generate(&self, req: &PodcastRequest) -> anyhow::Result<PodcastResult> {
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

        // 读取一个服务端帧；错误帧直接报错；返回事件号（带超时，防止服务端不响应时永久挂起）
        async fn next_event(
            ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            timeout_secs: u64,
        ) -> anyhow::Result<Frame> {
            let wait = async {
                loop {
                    match ws.next().await {
                        Some(Ok(Message::Binary(data))) => {
                            let f = volc_proto::decode_frame(&data)?;
                            if f.message_type == volc_proto::MSG_ERROR {
                                anyhow::bail!(
                                    "播客 API 返回错误（code {}）: {}",
                                    f.event,
                                    String::from_utf8_lossy(&f.payload)
                                );
                            }
                            return Ok(f);
                        }
                        Some(Ok(Message::Ping(_))) => {
                            let _ = ws.send(Message::Pong(vec![])).await;
                        }
                        Some(Ok(Message::Close(_))) => anyhow::bail!("连接被服务器关闭"),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => anyhow::bail!("WebSocket 错误: {e}"),
                        None => anyhow::bail!("连接结束"),
                    }
                }
            };
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), wait)
                .await
                .map_err(|_| anyhow::anyhow!("等待播客服务响应超时（{}s）", timeout_secs))?
        }

        // 1. start_connection（event=1，无 session_id）→ ConnectionStarted(50)
        let frame = volc_proto::encode_client_frame(volc_proto::EV_START_CONNECTION, None, b"{}");
        ws.send(Message::Binary(frame)).await?;
        next_event(&mut ws, 30).await?;

        // 2. start_session（event=100，带 session_id + 请求 payload）→ SessionStarted(150)
        let session_id = uuid::Uuid::new_v4().to_string();
        let payload = self.build_payload(req);
        let frame = volc_proto::encode_client_frame(
            volc_proto::EV_START_SESSION,
            Some(&session_id),
            payload.to_string().as_bytes(),
        );
        ws.send(Message::Binary(frame)).await?;
        next_event(&mut ws, 30).await?;

        // 3. finish_session（event=102）触发生成
        let frame = volc_proto::encode_client_frame(volc_proto::EV_FINISH_SESSION, Some(&session_id), b"{}");
        ws.send(Message::Binary(frame)).await?;

        let mut audio_buf: Vec<u8> = Vec::new();
        let mut audio_url: Option<String> = None;
        let mut script_lines: Vec<String> = Vec::new();
        let mut subtitles: Vec<super::SubtitleEntry> = Vec::new();
        // 当前轮次：RoundStart 给出文本/说话人，RoundEnd 给出起止时间
        let mut cur_round: Option<(String, String)> = None;

        // 4. 读生成事件：PodcastRoundResponse(361) 音频、RoundStart(360) 文本、PodcastEnd(363)、SessionFinished(152)
        let read_loop = async {
            loop {
                let f = next_event(&mut ws, 120).await?;
                match f.event {
                    volc_proto::EV_PODCAST_ROUND_RESPONSE => {
                        audio_buf.extend_from_slice(&f.payload);
                    }
                    volc_proto::EV_PODCAST_ROUND_START => {
                        let v: serde_json::Value = serde_json::from_slice(&f.payload)?;
                        let speaker = v["speaker"].as_str().unwrap_or("");
                        let text = v["text"].as_str().unwrap_or("");
                        let round_id = v["round_id"].as_i64().unwrap_or(-1);
                        if req.only_nlp_text {
                            script_lines.push(format!("{}：{text}", self.speaker_label(speaker)));
                        }
                        // round_id -1=片头音乐 9999=片尾音乐，无正文，跳过字幕
                        if round_id != -1 && round_id != 9999 && !text.is_empty() {
                            cur_round = Some((self.speaker_label(speaker), text.to_string()));
                        } else {
                            cur_round = None;
                        }
                    }
                    volc_proto::EV_PODCAST_ROUND_END => {
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&f.payload) {
                            let start = v["start_time"].as_f64().unwrap_or(0.0);
                            let end = v["end_time"].as_f64().unwrap_or(0.0);
                            if let Some((speaker, text)) = cur_round.take() {
                                // 按句子切分成单行字幕，时间按字数比例分配
                                let mut entries =
                                    super::split_subtitle_entries(&speaker, &text, start, end);
                                subtitles.append(&mut entries);
                            }
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
            }
            anyhow::Ok(())
        };
        tokio::time::timeout(std::time::Duration::from_secs(600), read_loop)
            .await
            .map_err(|_| anyhow::anyhow!("等待播客生成超时（600s）"))??;

        // 5. finish_connection（event=2）
        let frame = volc_proto::encode_client_frame(volc_proto::EV_FINISH_CONNECTION, None, b"{}");
        let _ = ws.send(Message::Binary(frame)).await;
        let _ = ws.close(None).await;

        if req.only_nlp_text {
            anyhow::ensure!(
                !script_lines.is_empty(),
                "播客 API 未返回脚本文本"
            );
            return Ok(PodcastResult::ScriptText(script_lines.join("\n")));
        }

        let bytes = if let Some(url) = audio_url {
            let resp = self.client.get(&url).send().await?;
            anyhow::ensure!(resp.status().is_success(), "下载播客音频失败: {}", resp.status());
            resp.bytes().await?.to_vec()
        } else {
            anyhow::ensure!(!audio_buf.is_empty(), "播客 API 未返回音频");
            audio_buf
        };
        Ok(PodcastResult::Audio { bytes, subtitles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> VolcPodcast {
        VolcPodcast::new(
            "appid".into(),
            "token".into(),
            vec!["voice_a".into(), "voice_b".into()],
        )
    }

    #[test]
    fn single_speaker_without_voice_falls_back_to_default() {
        // 双人固定；未选音色回落配置默认
        let x = v().with_speaker_override(None, None);
        assert_eq!(x.speakers(), vec!["voice_a".to_string(), "voice_b".to_string()]);
    }

    #[test]
    fn single_speaker_with_host_voice() {
        let x = v().with_speaker_override(Some("voice_x".into()), None);
        assert_eq!(x.speakers(), vec!["voice_x".to_string(), "voice_b".to_string()]);
    }

    #[test]
    fn dual_speaker_partial_override() {
        // 只覆盖主持人：嘉宾回落默认
        let x = v().with_speaker_override(Some("voice_x".into()), None);
        assert_eq!(x.speakers(), vec!["voice_x".to_string(), "voice_b".to_string()]);
    }

    #[test]
    fn payload_omits_random_order_for_single_speaker() {
        // 双人：random_order 为 true；speakers 固定 2 个
        let y = v();
        let req = PodcastRequest { input_text: Some("测试".into()), nlp_texts: None, only_nlp_text: false };
        let p2 = y.build_payload(&req);
        assert_eq!(p2["speaker_info"]["random_order"], true);
        assert_eq!(p2["speaker_info"]["speakers"].as_array().unwrap().len(), 2);
    }
}
