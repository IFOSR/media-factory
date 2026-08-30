//! 火山播客 API WebSocket v3 二进制帧编解码（对照官方 Python SDK protocols.py 精确实现）。
//!
//! header（4 字节）：
//! - byte0: (version=1 << 4) | header_size=1  => 0x11
//! - byte1: (msg_type << 4) | flag
//! - byte2: (serialization << 4) | compression
//! - byte3: 保留 0x00
//!
//! 客户端帧（FullClientRequest, flag=WithEvent）：
//!   [header][event i32][session_id_len u32][session_id][payload_size u32][payload]
//!   其中 session_id 在 connection 类事件（1/2）时省略。
//!
//! 服务端帧（FullServerResponse/AudioOnlyServer, flag=WithEvent）：
//!   [header][event i32][session_id?][connect_id?][payload_size u32][payload]
//!   - session_id 在事件 1/2/50/51/52 时省略
//!   - connect_id 仅在事件 50/51/52 时存在
//!
//! 错误帧（Error）：
//!   [header][error_code u32][payload_size u32][payload]
//!
//! 整数大端，event 为 i32。

// 协议面常量不全部被当前代码引用，保留作为协议文档与后续实现参考。
#![allow(dead_code)]

use flate2::read::GzDecoder;
use std::io::Read;

// message type（byte1 左 4-bit）
pub const MSG_FULL_CLIENT_REQUEST: u8 = 0b0001;
pub const MSG_FULL_SERVER_RESPONSE: u8 = 0b1001;
pub const MSG_AUDIO_ONLY_SERVER: u8 = 0b1011;
pub const MSG_ERROR: u8 = 0b1111;
pub const FLAG_WITH_EVENT: u8 = 0b0100;
pub const SER_JSON: u8 = 0x10;
pub const COMPRESSION_GZIP: u8 = 0x01;

// 上行 event type
pub const EV_START_CONNECTION: u32 = 1;
pub const EV_FINISH_CONNECTION: u32 = 2;
pub const EV_START_SESSION: u32 = 100;
pub const EV_CANCEL_SESSION: u32 = 101;
pub const EV_FINISH_SESSION: u32 = 102;

// 下行 event type
pub const EV_CONNECTION_STARTED: u32 = 50;
pub const EV_CONNECTION_FAILED: u32 = 51;
pub const EV_CONNECTION_FINISHED: u32 = 52;
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
    pub payload: Vec<u8>,
}

/// session_id 是否在该事件中被省略（connection 类事件）
fn session_id_skipped(event: u32) -> bool {
    matches!(
        event,
        EV_START_CONNECTION
            | EV_FINISH_CONNECTION
            | EV_CONNECTION_STARTED
            | EV_CONNECTION_FAILED
            | EV_CONNECTION_FINISHED
    )
}

/// connect_id 是否在该事件中存在（connection 下行事件）
fn connect_id_present(event: u32) -> bool {
    matches!(
        event,
        EV_CONNECTION_STARTED | EV_CONNECTION_FAILED | EV_CONNECTION_FINISHED
    )
}

/// 编码客户端帧（FullClientRequest + WithEvent）
pub fn encode_client_frame(event: u32, session_id: Option<&str>, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![
        0x11,
        (MSG_FULL_CLIENT_REQUEST << 4) | FLAG_WITH_EVENT,
        SER_JSON,
        0x00,
    ];
    out.extend_from_slice(&(event as i32).to_be_bytes());
    if !session_id_skipped(event) {
        let sid = session_id.unwrap_or("");
        out.extend_from_slice(&(sid.len() as u32).to_be_bytes());
        out.extend_from_slice(sid.as_bytes());
    }
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
    let ser_byte = bytes[2];
    let compressed = (ser_byte & 0x0F) == COMPRESSION_GZIP;

    // 错误帧：[header][error_code u32][payload_size u32][payload]
    if message_type == MSG_ERROR {
        let mut pos = 4usize;
        let code = take_u32(bytes, &mut pos)?;
        let plen = take_u32(bytes, &mut pos)? as usize;
        let payload = take_bytes(bytes, &mut pos, plen)?;
        return Ok(Frame {
            message_type,
            event: code,
            payload,
        });
    }

    let mut pos = 4usize;
    let event = take_u32(bytes, &mut pos)?;

    // session_id（connection 类事件省略）
    if !session_id_skipped(event) {
        let id_len = take_u32(bytes, &mut pos)? as usize;
        let _session_id = take_bytes(bytes, &mut pos, id_len)?;
    }

    // connect_id（仅 connection 下行事件）
    if connect_id_present(event) {
        let cid_len = take_u32(bytes, &mut pos)? as usize;
        let _connect_id = take_bytes(bytes, &mut pos, cid_len)?;
    }

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
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_connection_frame_no_session_id() {
        let f = encode_client_frame(EV_START_CONNECTION, None, b"{}");
        assert_eq!(&f[0..4], &[0x11, 0x14, 0x10, 0x00]);
        assert_eq!(&f[4..8], &1u32.to_be_bytes());
        assert_eq!(&f[8..12], &2u32.to_be_bytes()); // payload size，无 session_id
        assert_eq!(&f[12..], b"{}");
    }

    #[test]
    fn start_session_frame_with_session_id() {
        let f = encode_client_frame(EV_START_SESSION, Some("sid123"), b"{\"action\":0}");
        assert_eq!(&f[0..4], &[0x11, 0x14, 0x10, 0x00]);
        assert_eq!(&f[4..8], &100u32.to_be_bytes());
        assert_eq!(&f[8..12], &6u32.to_be_bytes()); // session_id len
        assert_eq!(&f[12..18], b"sid123");
        assert_eq!(&f[18..22], &12u32.to_be_bytes()); // payload size
        assert_eq!(&f[22..], br#"{"action":0}"#);
    }

    #[test]
    fn connection_started_response_decodes_connect_id() {
        let mut f = Vec::new();
        f.extend_from_slice(&[0x11, 0x94, 0x10, 0x00]);
        f.extend_from_slice(&EV_CONNECTION_STARTED.to_be_bytes());
        f.extend_from_slice(&(4u32).to_be_bytes()); // connect_id len
        f.extend_from_slice(b"cid1");
        f.extend_from_slice(&(5u32).to_be_bytes()); // payload size
        f.extend_from_slice(b"hello");

        let d = decode_frame(&f).unwrap();
        assert_eq!(d.message_type, MSG_FULL_SERVER_RESPONSE);
        assert_eq!(d.event, EV_CONNECTION_STARTED);
        assert_eq!(d.payload, b"hello");
    }

    #[test]
    fn session_started_response_decodes_session_id() {
        let mut f = Vec::new();
        f.extend_from_slice(&[0x11, 0x94, 0x10, 0x00]);
        f.extend_from_slice(&EV_SESSION_STARTED.to_be_bytes());
        f.extend_from_slice(&(6u32).to_be_bytes()); // session_id len
        f.extend_from_slice(b"sid123");
        f.extend_from_slice(&(5u32).to_be_bytes()); // payload size
        f.extend_from_slice(b"hello");

        let d = decode_frame(&f).unwrap();
        assert_eq!(d.event, EV_SESSION_STARTED);
        assert_eq!(d.payload, b"hello");
    }

    #[test]
    fn error_frame_decodes() {
        let mut f = Vec::new();
        f.extend_from_slice(&[0x11, 0xF0, 0x10, 0x00]);
        f.extend_from_slice(&45000000u32.to_be_bytes());
        f.extend_from_slice(&(19u32).to_be_bytes());
        f.extend_from_slice(br#"{"error":"bad req"}"#);

        let d = decode_frame(&f).unwrap();
        assert_eq!(d.message_type, MSG_ERROR);
        assert_eq!(d.event, 45000000);
        assert_eq!(d.payload, br#"{"error":"bad req"}"#);
    }
}
