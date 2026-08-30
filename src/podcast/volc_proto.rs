//! 火山播客 API WebSocket v3 二进制帧编解码。
//!
//! 帧结构（自官方文档推导）：
//! `[4字节 header][4字节 event_type u32 BE][4字节 session_id_len u32 BE][session_id][4字节 payload_len u32 BE][payload]`
//!
//! header：
//! - byte0: 0x11（protocol v1 + 4 字节 header）
//! - byte1: 左 4-bit = message type，右 4-bit = flags（0b0100 = 带 event number）
//! - byte2: 左 4-bit = 序列化（0b0001 JSON / 0b0000 Raw），右 4-bit = 压缩（0b0000 无 / 0b0001 gzip）
//! - byte3: 保留 0x00

use flate2::read::GzDecoder;
use std::io::Read;

pub const HDR: u8 = 0x11;

// message type（byte1 左 4-bit）
pub const MSG_FULL_CLIENT_REQUEST: u8 = 0b1001;
pub const MSG_AUDIO_ONLY_RESPONSE: u8 = 0b1011;
pub const MSG_OTHER_RESPONSE: u8 = 0b1001;
pub const MSG_ERROR: u8 = 0b1111;

pub const FLAG_WITH_EVENT: u8 = 0b0100;
pub const SER_JSON: u8 = 0x10;
pub const COMPRESSION_GZIP: u8 = 0x01;

// 上行 event type
pub const EV_START_SESSION: u32 = 0;
pub const EV_FINISH_SESSION: u32 = 1;
pub const EV_FINISH_CONNECTION: u32 = 2;

// 下行 event type
pub const EV_SESSION_STARTED: u32 = 150;
pub const EV_SESSION_FINISHED: u32 = 152;
pub const EV_USAGE_RESPONSE: u32 = 154;
pub const EV_PODCAST_ROUND_START: u32 = 360;
pub const EV_PODCAST_ROUND_RESPONSE: u32 = 361;
pub const EV_PODCAST_ROUND_END: u32 = 362;
pub const EV_PODCAST_END: u32 = 363;

#[derive(Debug, Clone)]
pub struct Frame {
    pub message_type: u8,
    pub event: u32,
    pub session_id: String,
    pub payload: Vec<u8>,
}

/// 编码客户端请求帧（full client request，带 event number）
pub fn encode_client_frame(event: u32, session_id: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(HDR);
    out.push((MSG_FULL_CLIENT_REQUEST << 4) | FLAG_WITH_EVENT);
    out.push(SER_JSON);
    out.push(0x00);
    out.extend_from_slice(&event.to_be_bytes());
    out.extend_from_slice(&(session_id.len() as u32).to_be_bytes());
    out.extend_from_slice(session_id.as_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn take_u32(bytes: &[u8], pos: &mut usize) -> anyhow::Result<u32> {
    anyhow::ensure!(bytes.len() >= *pos + 4, "帧不完整：缺 u32 字段");
    let v = u32::from_be_bytes(bytes[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

fn take_bytes(bytes: &[u8], pos: &mut usize, len: usize) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(bytes.len() >= *pos + len, "帧不完整：缺 {len} 字节");
    let v = bytes[*pos..*pos + len].to_vec();
    *pos += len;
    Ok(v)
}

/// 解码服务端帧
pub fn decode_frame(bytes: &[u8]) -> anyhow::Result<Frame> {
    anyhow::ensure!(bytes.len() >= 4, "帧太短");
    let message_type = bytes[1] >> 4;
    let flags = bytes[1] & 0x0F;
    let ser_byte = bytes[2];
    let compressed = (ser_byte & 0x0F) == COMPRESSION_GZIP;

    // 错误帧：byte1 = 0xF0，[4~7]=错误码，其后为错误消息
    if message_type == MSG_ERROR {
        let mut pos = 4usize;
        let code = take_u32(bytes, &mut pos)?;
        let payload = bytes[pos..].to_vec();
        return Ok(Frame {
            message_type,
            event: code,
            session_id: String::new(),
            payload,
        });
    }

    let mut pos = 4usize;
    let event = if flags & FLAG_WITH_EVENT != 0 {
        take_u32(bytes, &mut pos)?
    } else {
        0
    };
    let sid_len = take_u32(bytes, &mut pos)? as usize;
    let session_id = String::from_utf8_lossy(&take_bytes(bytes, &mut pos, sid_len)?).to_string();
    let plen = take_u32(bytes, &mut pos)? as usize;
    let mut payload = take_bytes(bytes, &mut pos, plen)?;

    if compressed {
        let mut d = GzDecoder::new(&payload[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out)?;
        payload = out;
    }

    Ok(Frame {
        message_type,
        event,
        session_id,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frame_roundtrip() {
        let frame = encode_client_frame(EV_START_SESSION, "abcd1234", br#"{"action":0}"#);
        assert_eq!(frame[0], 0x11);
        assert_eq!(frame[1], 0x94); // 0b1001_0100
        assert_eq!(frame[2], 0x10);
        assert_eq!(frame[3], 0x00);
        // 解码（客户端帧也能被 decode_frame 解析，结构一致）
        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.message_type, MSG_FULL_CLIENT_REQUEST);
        assert_eq!(decoded.event, EV_START_SESSION);
        assert_eq!(decoded.session_id, "abcd1234");
        assert_eq!(decoded.payload, br#"{"action":0}"#);
    }

    #[test]
    fn server_audio_frame_decodes() {
        // 构造一帧服务端音频响应（event 361）
        let mut frame = Vec::new();
        frame.push(HDR);
        frame.push((MSG_AUDIO_ONLY_RESPONSE << 4) | FLAG_WITH_EVENT);
        frame.push(0x00); // Raw, no compression
        frame.push(0x00);
        frame.extend_from_slice(&EV_PODCAST_ROUND_RESPONSE.to_be_bytes());
        frame.extend_from_slice(&(8u32).to_be_bytes());
        frame.extend_from_slice(b"abcd1234");
        frame.extend_from_slice(&(5u32).to_be_bytes());
        frame.extend_from_slice(b"MP3XX");

        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.message_type, MSG_AUDIO_ONLY_RESPONSE);
        assert_eq!(decoded.event, EV_PODCAST_ROUND_RESPONSE);
        assert_eq!(decoded.payload, b"MP3XX");
    }

    #[test]
    fn error_frame_decodes() {
        let mut frame = Vec::new();
        frame.push(HDR);
        frame.push(0xF0); // 0b1111_0000
        frame.push(0x10);
        frame.push(0x00);
        frame.extend_from_slice(&1001u32.to_be_bytes());
        frame.extend_from_slice(br#"{"error":"bad request"}"#);

        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.message_type, MSG_ERROR);
        assert_eq!(decoded.event, 1001);
        assert_eq!(decoded.payload, br#"{"error":"bad request"}"#);
    }
}
