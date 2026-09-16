// Bytecode file header and magic bytes specification.

// Magic bytes at the start of compiled bytecode binary
pub const MAGIC: &[u8; 7] = b"neyuki!";
pub const BYTECODE_VERSION: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeHeader {
    pub version: u8,
}

impl BytecodeHeader {
    pub fn new() -> Self {
        Self {
            version: BYTECODE_VERSION,
        }
    }

    pub fn write_to(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(MAGIC);
        buf.push(self.version);
    }

    pub fn parse(bytes: &[u8]) -> Result<(Self, usize), String> {
        if bytes.len() < 8 {
            return Err("bytecode buffer too small to contain header".to_string());
        }
        if &bytes[0..7] != MAGIC {
            return Err("invalid bytecode magic bytes: expected 'neyuki!'".to_string());
        }
        let version = bytes[7];
        if version != BYTECODE_VERSION {
            return Err(format!(
                "unsupported bytecode version: {} (expected {})",
                version, BYTECODE_VERSION
            ));
        }
        Ok((BytecodeHeader { version }, 8))
    }
}

impl Default for BytecodeHeader {
    fn default() -> Self {
        Self::new()
    }
}
