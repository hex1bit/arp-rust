use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::message::Message;
use crate::error::{Error, Result};

pub struct MessageCodec;
const MAX_MESSAGE_SIZE: usize = 8 * 1024 * 1024;

impl Encoder<Message> for MessageCodec {
    type Error = Error;

    fn encode(&mut self, msg: Message, dst: &mut BytesMut) -> Result<()> {
        let json = serde_json::to_vec(&msg)
            .map_err(|e| Error::Codec(format!("Failed to serialize message: {}", e)))?;
        if json.len() > MAX_MESSAGE_SIZE {
            return Err(Error::Codec(format!(
                "Message too large: {} > {}",
                json.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        let type_byte = msg.type_byte();

        if dst.remaining_mut() < 1 + 8 + json.len() {
            dst.reserve(1 + 8 + json.len());
        }

        dst.put_u8(type_byte);
        dst.put_u64(json.len() as u64);
        dst.put_slice(&json);

        Ok(())
    }
}

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Message>> {
        if src.len() < 9 {
            return Ok(None);
        }

        let mut length_bytes = [0u8; 8];
        length_bytes.copy_from_slice(&src[1..9]);
        let length = u64::from_be_bytes(length_bytes) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(Error::Protocol(format!(
                "message length {} exceeds max {}",
                length, MAX_MESSAGE_SIZE
            )));
        }

        if src.len() < 9 + length {
            src.reserve(9 + length - src.len());
            return Ok(None);
        }

        let type_byte = src[0];
        src.advance(9);
        let json_bytes = src.split_to(length);

        let msg = serde_json::from_slice(&json_bytes).map_err(|e| {
            Error::Codec(format!(
                "Failed to deserialize message (type: {:?}): {}",
                Message::from_type_byte(type_byte),
                e
            ))
        })?;

        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::message::{LoginMsg, PingMsg};

    #[test]
    fn test_codec_ping() {
        let mut codec = MessageCodec;
        let msg = Message::Ping(PingMsg { timestamp: 123456 });

        let mut buf = BytesMut::new();
        codec.encode(msg.clone(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();

        match decoded {
            Message::Ping(ping) => assert_eq!(ping.timestamp, 123456),
            _ => panic!("Expected Ping message"),
        }
    }

    #[test]
    fn test_codec_login() {
        let mut codec = MessageCodec;
        let msg = Message::Login(LoginMsg {
            version: "0.1.0".to_string(),
            hostname: "test-host".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            user: "test-user".to_string(),
            timestamp: 1234567890,
            privilege_key: "test-key".to_string(),
            run_id: "".to_string(),
            pool_count: 5,
        });

        let mut buf = BytesMut::new();
        codec.encode(msg.clone(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();

        match decoded {
            Message::Login(login) => {
                assert_eq!(login.version, "0.1.0");
                assert_eq!(login.hostname, "test-host");
                assert_eq!(login.pool_count, 5);
            }
            _ => panic!("Expected Login message"),
        }
    }

    #[test]
    fn test_codec_partial_message() {
        let mut codec = MessageCodec;
        let msg = Message::Ping(PingMsg { timestamp: 123456 });

        let mut buf = BytesMut::new();
        codec.encode(msg, &mut buf).unwrap();

        let mut partial = buf.split_to(5);
        assert!(codec.decode(&mut partial).unwrap().is_none());

        partial.unsplit(buf);
        assert!(codec.decode(&mut partial).unwrap().is_some());
    }

    #[test]
    fn test_codec_rejects_too_large_message() {
        let mut codec = MessageCodec;
        let mut buf = BytesMut::new();
        buf.put_u8(b'p');
        buf.put_u64((MAX_MESSAGE_SIZE as u64) + 1);
        let err = codec.decode(&mut buf).unwrap_err();
        match err {
            Error::Protocol(msg) => assert!(msg.contains("exceeds max")),
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
