// Raw binary buffer structure and memory manipulation for Neyuki VM.

use std::convert::TryInto;

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

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn check_bounds(&self, offset: usize, size: usize) -> Result<(), String> {
        if offset + size > self.data.len() {
            Err(format!(
                "buffer out of bounds: offset {} size {} buffer length {}",
                offset,
                size,
                self.data.len()
            ))
        } else {
            Ok(())
        }
    }

    pub fn read_u8(&self, offset: usize) -> Result<u8, String> {
        self.check_bounds(offset, 1)?;
        Ok(self.data[offset])
    }

    pub fn write_u8(&mut self, offset: usize, val: u8) -> Result<(), String> {
        self.check_bounds(offset, 1)?;
        self.data[offset] = val;
        Ok(())
    }

    pub fn read_i8(&self, offset: usize) -> Result<i8, String> {
        self.check_bounds(offset, 1)?;
        Ok(self.data[offset] as i8)
    }

    pub fn write_i8(&mut self, offset: usize, val: i8) -> Result<(), String> {
        self.check_bounds(offset, 1)?;
        self.data[offset] = val as u8;
        Ok(())
    }

    pub fn read_u16(&self, offset: usize) -> Result<u16, String> {
        self.check_bounds(offset, 2)?;
        let bytes: [u8; 2] = self.data[offset..offset + 2].try_into().unwrap();
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn write_u16(&mut self, offset: usize, val: u16) -> Result<(), String> {
        self.check_bounds(offset, 2)?;
        self.data[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_i16(&self, offset: usize) -> Result<i16, String> {
        self.check_bounds(offset, 2)?;
        let bytes: [u8; 2] = self.data[offset..offset + 2].try_into().unwrap();
        Ok(i16::from_le_bytes(bytes))
    }

    pub fn write_i16(&mut self, offset: usize, val: i16) -> Result<(), String> {
        self.check_bounds(offset, 2)?;
        self.data[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_u32(&self, offset: usize) -> Result<u32, String> {
        self.check_bounds(offset, 4)?;
        let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().unwrap();
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn write_u32(&mut self, offset: usize, val: u32) -> Result<(), String> {
        self.check_bounds(offset, 4)?;
        self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_i32(&self, offset: usize) -> Result<i32, String> {
        self.check_bounds(offset, 4)?;
        let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().unwrap();
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn write_i32(&mut self, offset: usize, val: i32) -> Result<(), String> {
        self.check_bounds(offset, 4)?;
        self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_f32(&self, offset: usize) -> Result<f32, String> {
        self.check_bounds(offset, 4)?;
        let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().unwrap();
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn write_f32(&mut self, offset: usize, val: f32) -> Result<(), String> {
        self.check_bounds(offset, 4)?;
        self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_f64(&self, offset: usize) -> Result<f64, String> {
        self.check_bounds(offset, 8)?;
        let bytes: [u8; 8] = self.data[offset..offset + 8].try_into().unwrap();
        Ok(f64::from_le_bytes(bytes))
    }

    pub fn write_f64(&mut self, offset: usize, val: f64) -> Result<(), String> {
        self.check_bounds(offset, 8)?;
        self.data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        Ok(())
    }

    pub fn read_string(&self, offset: usize, count: usize) -> Result<String, String> {
        self.check_bounds(offset, count)?;
        String::from_utf8(self.data[offset..offset + count].to_vec())
            .map_err(|e| format!("invalid utf-8 string in buffer: {}", e))
    }

    pub fn write_string(&mut self, offset: usize, s: &str) -> Result<(), String> {
        let bytes = s.as_bytes();
        self.check_bounds(offset, bytes.len())?;
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub fn copy(
        &mut self,
        target_offset: usize,
        source: &VmBuffer,
        source_offset: usize,
        count: usize,
    ) -> Result<(), String> {
        self.check_bounds(target_offset, count)?;
        source.check_bounds(source_offset, count)?;
        self.data[target_offset..target_offset + count]
            .copy_from_slice(&source.data[source_offset..source_offset + count]);
        Ok(())
    }

    pub fn fill(&mut self, offset: usize, val: u8, count: usize) -> Result<(), String> {
        self.check_bounds(offset, count)?;
        for b in &mut self.data[offset..offset + count] {
            *b = val;
        }
        Ok(())
    }
}
