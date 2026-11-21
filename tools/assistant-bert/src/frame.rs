use std::io;
use std::io::Read;
use std::io::Write;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::protocol::qa_frame_len;

pub fn read<T: DeserializeOwned>(reader: &mut impl Read, max: usize) -> io::Result<Option<T>> {
    let mut header = [0; 4];
    loop {
        match reader.read(&mut header[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    reader.read_exact(&mut header[1..])?;
    let len = qa_frame_len(header)?;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid QA worker frame size",
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid QA worker frame"))
}

pub fn write(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    let header = u32::try_from(payload.len())
        .map_err(io::Error::other)?
        .to_be_bytes();
    qa_frame_len(header)?;
    writer.write_all(&header)?;
    writer.write_all(&payload)?;
    writer.flush()
}
