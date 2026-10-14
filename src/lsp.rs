//! A language server over stdio: JSON-RPC messages framed by `Content-Length`,
//! as the Language Server Protocol specifies. Reading, writing and draining
//! stderr each happen on a thread of their own, so a server that stalls or
//! floods a pipe can never freeze the editor.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How many of the server's last stderr lines are kept, to say why it stopped.
const LOG_LINES: usize = 20;

/// How long a server gets to exit after being asked before it is killed.
const EXIT_GRACE: Duration = Duration::from_millis(500);

pub struct Server {
    child: Child,

    outgoing: Option<Sender<Vec<u8>>>,

    incoming: Receiver<Value>,

    next_id: i64,

    log: Arc<Mutex<VecDeque<String>>>,
}

/// The server's output ended: it exited or crashed.
#[derive(Debug)]
pub struct Stopped;

impl Server {
    pub fn spawn(program: &Path, args: &[&str], cwd: &Path) -> io::Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin.take().expect("stdin is piped");

        let stdout = child.stdout.take().expect("stdout is piped");

        let stderr = child.stderr.take().expect("stderr is piped");

        let (to_server, outbox) = mpsc::channel::<Vec<u8>>();

        thread::spawn(move || {
            for frame in outbox {
                if stdin.write_all(&frame).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });

        let (inbox, incoming) = mpsc::channel();

        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);

            while let Ok(Some(message)) = read_message(&mut reader) {
                if inbox.send(message).is_err() {
                    break;
                }
            }
        });

        let log = Arc::new(Mutex::new(VecDeque::new()));

        let sink = Arc::clone(&log);

        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut log = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

                if log.len() == LOG_LINES {
                    log.pop_front();
                }

                log.push_back(line);
            }
        });

        Ok(Self { child, outgoing: Some(to_server), incoming, next_id: 0, log })
    }

    fn send(&self, message: &Value) {
        if let Some(outgoing) = &self.outgoing {
            // A closed channel means the writer saw the pipe break; the reader
            // reports the same thing as `Stopped`, which is where it is handled.
            let _ = outgoing.send(frame(message));
        }
    }

    /// Send a request and return its id, to match the response against.
    pub fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;

        self.send(&json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params }));

        self.next_id
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Answer a request the server made of us.
    pub fn respond(&self, id: Value, result: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    /// Every message that has arrived, without waiting for more.
    pub fn poll(&self) -> Result<Vec<Value>, Stopped> {
        let mut messages = Vec::new();

        loop {
            match self.incoming.try_recv() {
                Ok(message) => messages.push(message),

                Err(TryRecvError::Empty) => return Ok(messages),

                // Hand over what did arrive; the next poll reports the stop.
                Err(TryRecvError::Disconnected) if !messages.is_empty() => return Ok(messages),

                Err(TryRecvError::Disconnected) => return Err(Stopped),
            }
        }
    }

    /// The server's last words on stderr.
    pub fn last_log(&self) -> Option<String> {
        let log = self.log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        log.iter().rev().find(|line| !line.trim().is_empty()).cloned()
    }

    /// Ask the server to shut down and exit, and kill it if it has not within
    /// a moment, so quitting never hangs on a busy server and never leaves one
    /// running behind.
    pub fn stop(mut self) {
        self.request("shutdown", Value::Null);

        self.notify("exit", Value::Null);

        // Dropping the sender ends the writer thread, which closes stdin.
        self.outgoing = None;

        let deadline = Instant::now() + EXIT_GRACE;

        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }

            thread::sleep(Duration::from_millis(10));
        }

        let _ = self.child.kill();

        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();

            let _ = self.child.wait();
        }
    }
}

fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();

    let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();

    frame.extend_from_slice(body.as_bytes());

    frame
}

/// Read one framed message. `Ok(None)` is a clean end of stream.
fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length: Option<usize> = None;

    let mut line = String::new();

    loop {
        line.clear();

        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }

        let header = line.trim_end();

        if header.is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }

    let length = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Content-Length"))?;

    let mut body = vec![0; length];

    reader.read_exact(&mut body)?;

    serde_json::from_slice(&body).map(Some).map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_read_back_one_by_one() {
        let first = json!({ "jsonrpc": "2.0", "id": 1, "result": { "text": "∀ x, x = x" } });

        let second = json!({ "jsonrpc": "2.0", "method": "$/lean/fileProgress", "params": {} });

        let mut stream = frame(&first);

        // Headers other than the length are allowed and ignored, in any case.
        stream.extend_from_slice(b"content-length: ");

        let body = second.to_string();

        stream.extend_from_slice(format!("{}\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n{body}", body.len()).as_bytes());

        let mut reader = io::Cursor::new(stream);

        assert_eq!(read_message(&mut reader).unwrap(), Some(first));

        assert_eq!(read_message(&mut reader).unwrap(), Some(second));

        assert_eq!(read_message(&mut reader).unwrap(), None, "a clean end of stream");
    }

    #[test]
    fn the_length_counts_bytes_not_chars() {
        let message = json!({ "goal": "⊢ 𝔸" });

        let framed = String::from_utf8(frame(&message)).unwrap();

        let body = message.to_string();

        assert!(framed.starts_with(&format!("Content-Length: {}\r\n\r\n", body.len())));

        assert!(body.len() > body.chars().count());
    }

    #[test]
    fn a_truncated_message_is_an_error_not_a_hang() {
        let mut reader = io::Cursor::new(b"Content-Length: 50\r\n\r\n{\"id\":1}".to_vec());

        assert!(read_message(&mut reader).is_err());
    }
}
