//! Line-delimited JSON over the overlay socket (the messages are in `sr_client::overlay_proto`).

use serde::Serialize;
use serde::de::DeserializeOwned;
use sr_client::OVERLAY_MAX_LINE;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

/// Write one message as one line.
pub fn send<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg).map_err(io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// Read one line into `buf`. `Ok(None)` at the end of the stream; a line longer than the limit is an error.
pub fn read_line(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<Option<()>> {
    buf.clear();
    let n = r.by_ref().take(OVERLAY_MAX_LINE as u64 + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > OVERLAY_MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "overlay message too long"));
    }
    Ok(Some(()))
}

/// A message from a line; anything that does not parse is dropped by the caller.
pub fn decode<T: DeserializeOwned>(line: &[u8]) -> Option<T> {
    serde_json::from_slice(line).ok()
}

/// `$XDG_RUNTIME_DIR/kettle-overlay-<pid>.sock` (the folder is private to the user).
pub fn socket_path(pid: u32) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join(format!("kettle-overlay-{pid}.sock")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_client::{FromHelper, ToHelper};

    #[test]
    fn lines_round_trip() {
        let mut wire = Vec::new();
        send(&mut wire, &FromHelper::SendChat { id: 5, text: "a\nb".into() }).unwrap();
        send(&mut wire, &ToHelper::Hide).unwrap();
        assert_eq!(wire.iter().filter(|b| **b == b'\n').count(), 2, "one line per message");
        let mut r = io::BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        assert!(read_line(&mut r, &mut buf).unwrap().is_some());
        assert_eq!(decode::<FromHelper>(&buf), Some(FromHelper::SendChat { id: 5, text: "a\nb".into() }));
        assert!(read_line(&mut r, &mut buf).unwrap().is_some());
        assert_eq!(decode::<ToHelper>(&buf), Some(ToHelper::Hide));
        assert!(read_line(&mut r, &mut buf).unwrap().is_none());
    }

    #[test]
    fn long_and_broken_lines() {
        let long = vec![b'x'; OVERLAY_MAX_LINE + 10];
        assert!(read_line(&mut io::BufReader::new(&long[..]), &mut Vec::new()).is_err());
        assert_eq!(decode::<FromHelper>(b"{oops"), None);
    }
}
