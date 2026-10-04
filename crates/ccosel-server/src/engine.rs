//! A game engine the server talks to over a line protocol (GTP for GNU Go; xboard for GNU
//! Chess when that comes): started for one question, given a time limit, and killed afterwards.
//! The server keeps no engine running between calls.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// Why a conversation with an engine ended early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// It couldn't be started at all: most likely it isn't installed.
    Start(String),
    /// It stopped answering by exiting.
    Crashed,
    TimedOut,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(why) => write!(f, "couldn't start it ({why}); is it installed?"),
            Self::Crashed => write!(f, "it stopped unexpectedly"),
            Self::TimedOut => write!(f, "it took too long and was stopped"),
        }
    }
}

/// One running engine. Dropping it kills the process.
pub struct LineEngine {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    deadline: Instant,
}

impl LineEngine {
    /// Start `argv`, to be answered within `limit` in all.
    pub fn start(argv: &[String], limit: Duration) -> Result<Self, EngineError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| EngineError::Start("no command".into()))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| EngineError::Start(e.to_string()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let (send, lines) = channel();
        // Its own thread, so a read can give up at the deadline: a blocking read can't.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
            deadline: Instant::now() + limit,
        })
    }

    pub fn send(&mut self, line: &str) -> Result<(), EngineError> {
        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .map_err(|_| EngineError::Crashed)
    }

    /// The next line it says, waiting no later than the deadline.
    pub fn read_line(&mut self) -> Result<String, EngineError> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        match self.lines.recv_timeout(left) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => Err(EngineError::TimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(EngineError::Crashed),
        }
    }
}

impl Drop for LineEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
