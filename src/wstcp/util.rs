use crate::errors::*;
use crypto::digest::Digest;
use crypto::sha1::Sha1;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug)]
pub struct WebSocketKey(pub String);

pub fn calc_accept_hash(key: &WebSocketKey) -> String {
    let mut buf: Vec<u8> = vec![0; 20]; // 160 bits
    let mut sh = Sha1::new();

    sh.input_str(&format!("{}{}", key.0, GUID));
    sh.result(buf.as_mut_slice());
    base64::encode(&buf)
}

impl From<std::io::Error> for Error {
    fn from(f: std::io::Error) -> Self {
        ErrorKind::WebSocket(format!("{}", f)).into()
    }
}
impl From<bytecodec::Error> for Error {
    fn from(f: bytecodec::Error) -> Self {
        ErrorKind::WebSocket(format!("{}", f)).into()
    }
}

impl From<bytecodec::ErrorKind> for Error {
    fn from(_f: bytecodec::ErrorKind) -> Self {
        // TODO: Improve error if needed.
        ErrorKind::WebSocket("bytecodec error".to_string()).into()
    }
}

pub fn error_encoder_full() -> bytecodec::Result<()> {
    Err(bytecodec::Error::from(bytecodec::ErrorKind::EncoderFull))
}
pub fn error_encoder_input() -> bytecodec::Result<()> {
    Err(bytecodec::Error::from(bytecodec::ErrorKind::InvalidInput))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn it_works() {
        let hash = calc_accept_hash(&WebSocketKey("dGhlIHNhbXBsZSBub25jZQ==".to_owned()));
        assert_eq!(hash, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }
}
