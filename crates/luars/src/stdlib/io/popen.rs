use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};

#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

pub(crate) struct ProcessExit {
    pub(crate) success: bool,
    pub(crate) kind: &'static str,
    pub(crate) code: i32,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PopenReader {
    child: Child,
    reader: BufReader<ChildStdout>,
    pushback: Option<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl PopenReader {
    fn new(child: Child, stdout: ChildStdout) -> Self {
        Self {
            child,
            reader: BufReader::new(stdout),
            pushback: None,
        }
    }

    pub(crate) fn read_line(&mut self, keep_newline: bool) -> io::Result<Option<String>> {
        let mut bytes = Vec::new();
        if let Some(byte) = self.pushback.take() {
            bytes.push(byte);
        }
        let count = self.reader.read_until(b'\n', &mut bytes)?;
        if bytes.is_empty() && count == 0 {
            return Ok(None);
        }
        if !keep_newline && bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "stream is not valid UTF-8"))
    }

    pub(crate) fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        if let Some(byte) = self.pushback.take() {
            bytes.push(byte);
        }
        self.reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn read_bytes(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(n);
        if let Some(byte) = self.pushback.take() {
            bytes.push(byte);
        }
        while bytes.len() < n {
            let mut buffer = vec![0u8; n - bytes.len()];
            let read = self.reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(bytes)
    }

    pub(crate) fn read_byte(&mut self) -> io::Result<Option<u8>> {
        if let Some(byte) = self.pushback.take() {
            return Ok(Some(byte));
        }

        let buffer = self.reader.fill_buf()?;
        if buffer.is_empty() {
            Ok(None)
        } else {
            let byte = buffer[0];
            self.reader.consume(1);
            Ok(Some(byte))
        }
    }

    pub(crate) fn unread_byte(&mut self, byte: u8) {
        self.pushback = Some(byte);
    }

    pub(crate) fn is_eof(&mut self) -> io::Result<bool> {
        if self.pushback.is_some() {
            return Ok(false);
        }
        Ok(self.reader.fill_buf()?.is_empty())
    }

    pub(crate) fn close(mut self) -> io::Result<ProcessExit> {
        self.pushback = None;
        map_exit_status(self.child.wait()?)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PopenWriter {
    child: Child,
    writer: BufWriter<ChildStdin>,
}

#[cfg(not(target_arch = "wasm32"))]
impl PopenWriter {
    fn new(child: Child, stdin: ChildStdin) -> Self {
        Self {
            child,
            writer: BufWriter::new(stdin),
        }
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    pub(crate) fn close(mut self) -> io::Result<ProcessExit> {
        self.writer.flush()?;
        drop(self.writer);
        map_exit_status(self.child.wait()?)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum PopenFile {
    Read(PopenReader),
    Write(PopenWriter),
}

#[cfg(not(target_arch = "wasm32"))]
fn map_exit_status(status: ExitStatus) -> io::Result<ProcessExit> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(code) = status.code() {
            return Ok(ProcessExit {
                success: status.success(),
                kind: "exit",
                code,
            });
        }

        if let Some(signal) = status.signal() {
            return Ok(ProcessExit {
                success: false,
                kind: "signal",
                code: signal,
            });
        }
    }

    Ok(ProcessExit {
        success: status.success(),
        kind: "exit",
        code: status.code().unwrap_or(-1),
    })
}

pub(crate) fn validate_popen_mode(mode: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = mode;
        false
    }

    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        matches!(mode, "r" | "w" | "rb" | "wb" | "rt" | "wt")
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
    {
        matches!(mode, "r" | "w")
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_popen(command: &str, mode: &str) -> io::Result<PopenFile> {
    let mode_char = mode
        .chars()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid mode"))?;

    let mut cmd = shell_command(command);
    match mode_char {
        'r' => {
            let mut child = cmd
                .stdin(Stdio::inherit())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("failed to capture process stdout"))?;
            Ok(PopenFile::Read(PopenReader::new(child, stdout)))
        }
        'w' => {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("failed to capture process stdin"))?;
            Ok(PopenFile::Write(PopenWriter::new(child, stdin)))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid mode")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_popen(_command: &str, _mode: &str) -> io::Result<()> {
    Err(io::Error::other("popen is not supported on wasm"))
}
