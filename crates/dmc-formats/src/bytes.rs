//! Bounds-checked, byte-order-aware reading and writing.

use crate::error::{FormatError, Result};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Endian {
    Big,
    Little,
}

impl Endian {
    pub fn other(self) -> Self {
        match self {
            Endian::Big => Endian::Little,
            Endian::Little => Endian::Big,
        }
    }
}

/// A read-only view over a byte slice. All offsets are absolute within the view.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    data: &'a [u8],
    pub endian: Endian,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], endian: Endian) -> Self {
        Self { data, endian }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn bytes(&self, offset: usize, len: usize) -> Result<&'a [u8]> {
        let oob = || FormatError::OutOfBounds {
            offset,
            len,
            size: self.data.len(),
        };
        let end = offset.checked_add(len).ok_or_else(oob)?;
        self.data.get(offset..end).ok_or_else(oob)
    }

    /// A reader over `offset..offset+len` whose offsets start again at zero.
    pub fn sub(&self, offset: usize, len: usize) -> Result<Reader<'a>> {
        Ok(Reader::new(self.bytes(offset, len)?, self.endian))
    }

    /// A reader over `offset..` whose offsets start again at zero.
    pub fn tail(&self, offset: usize) -> Result<Reader<'a>> {
        let len = self
            .data
            .len()
            .checked_sub(offset)
            .ok_or(FormatError::OutOfBounds {
                offset,
                len: 0,
                size: self.data.len(),
            })?;
        self.sub(offset, len)
    }

    fn array<const N: usize>(&self, offset: usize) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.bytes(offset, N)?);
        Ok(out)
    }

    pub fn u8(&self, offset: usize) -> Result<u8> {
        Ok(self.array::<1>(offset)?[0])
    }

    pub fn u16(&self, offset: usize) -> Result<u16> {
        let b = self.array(offset)?;
        Ok(match self.endian {
            Endian::Big => u16::from_be_bytes(b),
            Endian::Little => u16::from_le_bytes(b),
        })
    }

    pub fn i16(&self, offset: usize) -> Result<i16> {
        Ok(self.u16(offset)? as i16)
    }

    pub fn u32(&self, offset: usize) -> Result<u32> {
        let b = self.array(offset)?;
        Ok(match self.endian {
            Endian::Big => u32::from_be_bytes(b),
            Endian::Little => u32::from_le_bytes(b),
        })
    }

    pub fn f32(&self, offset: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32(offset)?))
    }

    pub fn vec3(&self, offset: usize) -> Result<[f32; 3]> {
        Ok([
            self.f32(offset)?,
            self.f32(offset + 4)?,
            self.f32(offset + 8)?,
        ])
    }

    /// A NUL-terminated string starting at `offset` (lossy ASCII).
    pub fn cstr(&self, offset: usize) -> Result<String> {
        let rest = self.tail(offset)?.data;
        let end = rest.iter().position(|&b| b == 0).ok_or_else(|| {
            FormatError::invalid("string", format!("no terminator after 0x{offset:x}"))
        })?;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

/// Growable little helper for producing binary data in a given byte order.
#[derive(Debug, Clone)]
pub struct Writer {
    pub buf: Vec<u8>,
    pub endian: Endian,
}

impl Writer {
    pub fn new(endian: Endian) -> Self {
        Self {
            buf: Vec::new(),
            endian,
        }
    }

    pub fn pos(&self) -> usize {
        self.buf.len()
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        let b = match self.endian {
            Endian::Big => v.to_be_bytes(),
            Endian::Little => v.to_le_bytes(),
        };
        self.bytes(&b)
    }

    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.u16(v as u16)
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        let b = self.u32_bytes(v);
        self.bytes(&b)
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }

    pub fn vec3(&mut self, v: [f32; 3]) -> &mut Self {
        self.f32(v[0]).f32(v[1]).f32(v[2])
    }

    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.buf.resize(self.buf.len() + n, 0);
        self
    }

    pub fn pad_to(&mut self, align: usize, fill: u8) -> &mut Self {
        while !self.buf.len().is_multiple_of(align) {
            self.buf.push(fill);
        }
        self
    }

    /// Overwrite a u32 already written at `offset` (for back-patching tables).
    pub fn set_u32(&mut self, offset: usize, v: u32) {
        let b = self.u32_bytes(v);
        self.buf[offset..offset + 4].copy_from_slice(&b);
    }

    fn u32_bytes(&self, v: u32) -> [u8; 4] {
        match self.endian {
            Endian::Big => v.to_be_bytes(),
            Endian::Little => v.to_le_bytes(),
        }
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_both_orders() {
        for e in [Endian::Big, Endian::Little] {
            let mut w = Writer::new(e);
            w.u8(7).u16(0x1234).u32(0xdead_beef).f32(1.5).i16(-2);
            let buf = w.finish();
            let r = Reader::new(&buf, e);
            assert_eq!(r.u8(0).unwrap(), 7);
            assert_eq!(r.u16(1).unwrap(), 0x1234);
            assert_eq!(r.u32(3).unwrap(), 0xdead_beef);
            assert_eq!(r.f32(7).unwrap(), 1.5);
            assert_eq!(r.i16(11).unwrap(), -2);
        }
    }

    #[test]
    fn out_of_bounds_is_an_error_not_a_panic() {
        let r = Reader::new(&[1, 2, 3], Endian::Big);
        assert!(matches!(r.u32(0), Err(FormatError::OutOfBounds { .. })));
        assert!(r.u32(usize::MAX - 1).is_err());
        assert!(r.tail(4).is_err());
        assert!(r.cstr(0).is_err());
    }
}
