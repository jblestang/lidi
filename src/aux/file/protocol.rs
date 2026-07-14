use std::{
    fmt, io,
    io::{Read, Write},
    string::FromUtf8Error,
};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    StringFormatError(FromUtf8Error),
    InvalidFileSize(usize, usize),
    InvalidHash(u128, u128),
    InvalidFileNameLen(usize),
}

pub const MAX_FILE_NAME_LEN: usize = 4096;

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Io(e) => write!(fmt, "I/O error: {e}"),
            Self::StringFormatError(e) => write!(fmt, "string format error: {e}"),
            Self::InvalidFileSize(s1, s2) => write!(fmt, "invalid file size: {s1} != {s2}"),
            Self::InvalidHash(h1, h2) => write!(fmt, "invalid hash: {h1:x} != {h2:x}"),
            Self::InvalidFileNameLen(len) => {
                write!(fmt, "invalid file name length: {len} > {MAX_FILE_NAME_LEN}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::StringFormatError(e) => Some(e),
            Self::InvalidFileSize(_, _) | Self::InvalidHash(_, _) | Self::InvalidFileNameLen(_) => {
                None
            }
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<FromUtf8Error> for Error {
    fn from(e: FromUtf8Error) -> Self {
        Self::StringFormatError(e)
    }
}

pub(crate) struct Header {
    pub(crate) file_name: String,
    pub(crate) mode: u32,
    pub(crate) file_length: u64,
}

impl Header {
    pub(crate) fn serialize_to<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&self.file_name.len().to_le_bytes())?;
        w.write_all(self.file_name.as_bytes())?;
        w.write_all(&self.mode.to_le_bytes())?;
        w.write_all(&self.file_length.to_le_bytes())?;
        Ok(())
    }

    pub(crate) fn deserialize_from<R: Read>(r: &mut R) -> Result<Self, Error> {
        let mut file_name_len = [0u8; 8];
        r.read_exact(&mut file_name_len)?;
        let file_name_len = usize::from_le_bytes(file_name_len);
        if file_name_len > MAX_FILE_NAME_LEN {
            return Err(Error::InvalidFileNameLen(file_name_len));
        }

        let mut file_name = vec![0; file_name_len];
        r.read_exact(&mut file_name)?;
        let file_name = String::from_utf8(file_name)?;

        let mut mode = [0u8; 4];
        r.read_exact(&mut mode)?;
        let mode = u32::from_le_bytes(mode);

        let mut file_length = [0u8; 8];
        r.read_exact(&mut file_length)?;
        let file_length = u64::from_le_bytes(file_length);

        Ok(Self {
            file_name,
            mode,
            file_length,
        })
    }
}

pub(crate) struct Footer {
    pub(crate) hash: u128,
}

impl Footer {
    pub fn serialize_to<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&self.hash.to_le_bytes())?;
        Ok(())
    }

    pub fn deserialize_from<R: Read>(r: &mut R) -> Result<Self, Error> {
        let mut hash = [0u8; 16];
        r.read_exact(&mut hash)?;
        let hash = u128::from_le_bytes(hash);

        Ok(Self { hash })
    }
}

#[cfg(test)]
mod repro {
    use super::{Error, Header, MAX_FILE_NAME_LEN};
    use std::io::Cursor;

    fn craft_header_bytes(file_name_len: usize) -> Vec<u8> {
        let mut bytes = file_name_len.to_le_bytes().to_vec();
        bytes.extend(std::iter::repeat_n(b'a', file_name_len));
        bytes.extend_from_slice(&0o644u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes
    }

    /// Unpatched master allocates `vec![0; file_name_len]` from attacker-controlled length.
    #[test]
    fn repro_huge_file_name_len_rejected() {
        let huge_len = MAX_FILE_NAME_LEN + 1;
        let mut cursor = Cursor::new(craft_header_bytes(huge_len));
        let result = Header::deserialize_from(&mut cursor);
        assert!(matches!(
            result,
            Err(Error::InvalidFileNameLen(len)) if len == huge_len
        ));
    }

    #[test]
    fn repro_max_file_name_len_still_accepted() {
        let mut cursor = Cursor::new(craft_header_bytes(MAX_FILE_NAME_LEN));
        match Header::deserialize_from(&mut cursor) {
            Ok(header) => assert_eq!(header.file_name.len(), MAX_FILE_NAME_LEN),
            Err(error) => panic!("max len should parse: {error}"),
        }
    }
}
