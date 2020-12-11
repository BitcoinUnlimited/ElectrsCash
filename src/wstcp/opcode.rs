#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    ContinuationFrame = 0x0,
    TextFrame = 0x1,
    BinaryFrame = 0x2,
    ConnectionClose = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}
impl Opcode {
    pub fn from_u8(n: u8) -> bytecodec::Result<Self> {
        Ok(match n {
            0x0 => Opcode::ContinuationFrame,
            0x1 => Opcode::TextFrame,
            0x2 => Opcode::BinaryFrame,
            0x8 => Opcode::ConnectionClose,
            0x9 => Opcode::Ping,
            0xA => Opcode::Pong,
            _ => return Err(bytecodec::ErrorKind::InvalidInput.into()),
        })
    }

    pub fn is_control(&self) -> bool {
        matches!(*self, Opcode::ConnectionClose | Opcode::Ping | Opcode::Pong)
    }
}
