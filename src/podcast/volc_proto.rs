//! 火山播客 API WebSocket v3 二进制帧编解码（据文档+服务器实测修正）。
//!
//! 客户端请求帧：`[header 4][event u32 BE][body_size u32 BE][body]`
//! - header = [0x11, 0x14, 0x10, 0x00]（protocol v1 / 4字节 header / message type 0b0001 / flags 0b0100 带 event / JSON / 无压缩）
//! - 首帧 event = 1（StartSession），结束帧 event = 2（FinishConnection）
//!
//! 服务端响应帧：`[header][event u32 BE][id_len u32 BE][id][body_size u32 BE][body]`
//! - 非音频响应 message type 0b1001，音频响应 message type 0b1011，错误帧 byte1=0xF0
//!
//! 整数大端。

// 协议面常量不全部被当前代码引用，保留作为协议文档与后续实现参考。
#![allow(dead_code)]

use flate2::read::GzDecoder;
use std::io::Read;

pub const HDR: u8 = 0x11;
pub const MSG_FULL_CLIENT_REQUEST: u8 = 0b0001;
pub const MSG_AUDIO_ONLY_RESPONSE: u8 = 0b1011;
pub const MSG_OTHER_RESPONSE: u8 = 0b1001;
pub const MSG_ERROR: u8 = 0b1111;
pub const FLAG_WITH_EVENT: u8 = 0b0100;
pub const SER_JSON: u8 = 0x10;
pub const COMPRESSION_GZIP: u8 = 0x01;

// 上行 event type
pub const EV_START_SESSION: u32 = 1;
pub const EV_FINISH_CONNECTION: u32 = 2;

// 下行 event type（SessionStarted 实测为 50，文档写 150 有误）
pub const EV_SESSION_STARTED: u32 = 50;
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

/// 编码客户端请求帧（full client request，带 event number）
pub fn encode_client_frame(event: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![
        HDR,
        (MSG_FULL_CLIENT_REQUEST << 4) | FLAG_WITH_EVENT,
        SER_JSON,
        0x00,
    ];
    out.extend_from_slice(&event.to_be_bytes());
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

/// 解码服务端帧：`[header][event][id_len][id][body_size][body]`
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
            payload,
        });
    }

    let mut pos = 4usize;
    let event = if flags & FLAG_WITH_EVENT != 0 {
        take_u32(bytes, &mut pos)?
    } else {
        0
    };
    let id_len = take_u32(bytes, &mut pos)? as usize;
    let _id = take_bytes(bytes, &mut pos, id_len)?;
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
    fn client_frame_encoding() {
        let frame = encode_client_frame(EV_START_SESSION, br#"{"action":0}"#);
        assert_eq!(&frame[0..4], &[0x11, 0x14, 0x10, 0x00]);
        assert_eq!(&frame[4..8], &1u32.to_be_bytes()); // event=StartSession(1)
        assert_eq!(&frame[8..12], &12u32.to_be_bytes()); // body size
        assert_eq!(&frame[12..], br#"{"action":0}"#);
    }

    #[test]
    fn server_frame_with_id_decodes() {
        let mut frame = Vec::new();
        frame.push(HDR);
        frame.push((MSG_OTHER_RESPONSE << 4) | FLAG_WITH_EVENT);
        frame.push(0x10);
        frame.push(0x00);
        frame.extend_from_slice(&EV_SESSION_STARTED.to_be_bytes());
        frame.extend_from_slice(&(4u32).to_be_bytes());
        frame.extend_from_slice(b"abcd");
        frame.extend_from_slice(&(5u32).to_be_bytes());
        frame.extend_from_slice(b"hello");

        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded.message_type, MSG_OTHER_RESPONSE);
        assert_eq!(decoded.event, EV_SESSION_STARTED);
        assert_eq!(decoded.payload, b"hello");
    }

    #[test]
    fn error_frame_decodes() {
        let mut frame = Vec::new();
        frame.push(HDR);
        frame.push(0xF0);
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
