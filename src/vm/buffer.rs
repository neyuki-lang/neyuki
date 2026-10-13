//! A fixed block of bytes, the value behind `buffer`.

#[derive(Clone, Debug, PartialEq)]
pub struct VmBuffer {
    pub data: Vec<u8>,
}

impl VmBuffer {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { data: bytes }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The byte range `offset..offset + size`, or an error when it does not
    /// fit inside the buffer.
    pub fn range(&self, offset: usize, size: usize) -> Result<std::ops::Range<usize>, String> {
        match offset.checked_add(size) {
            Some(end) if end <= self.data.len() => Ok(offset..end),
            _ => Err(format!(
                "buffer access out of bounds: offset {} size {} in a buffer of {} bytes",
                offset,
                size,
                self.data.len()
            )),
        }
    }

    pub fn read_u8(&self, offset: usize) -> Result<u8, String> {
        Ok(self.data[self.range(offset, 1)?][0])
    }

    pub fn write_u8(&mut self, offset: usize, val: u8) -> Result<(), String> {
        let range = self.range(offset, 1)?;
        self.data[range][0] = val;
        Ok(())
    }
}
