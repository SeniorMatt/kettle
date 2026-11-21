//! Client side of the socket: one request, one reply, in order.

use crate::proto::Reply;
use crate::proto::{ReplyLine, Request, RequestLine, read_line_limited};
use std::io::{self, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub struct Client {
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    pub fn connect(path: &Path, timeout: Duration) -> io::Result<Client> {
        let s = UnixStream::connect(path)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        Ok(Client { reader: BufReader::new(s), next_id: 1 })
    }

    pub fn call(&mut self, req: &Request) -> io::Result<Reply> {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = serde_json::to_string(&RequestLine { id, req: req.clone() }).map_err(io::Error::other)?;
        line.push('\n');
        self.reader.get_mut().write_all(line.as_bytes())?;
        loop {
            let Some(text) = read_line_limited(&mut self.reader)? else {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "host closed the socket"));
            };
            let rep: ReplyLine = serde_json::from_str(&text).map_err(io::Error::other)?;
            if rep.id == id {
                return Ok(rep.rep);
            }
            // A late answer to an earlier, timed-out request: skip it.
        }
    }
}
