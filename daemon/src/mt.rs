//! Translation sidecar client (daemon/scripts/mt_server.py).
//!
//! The sidecar keeps the CTranslate2 models resident; here we only speak
//! JSON-lines over stdin/stdout. This is the pattern we would use in production
//! until MT is ported to native Rust with `ct2rs`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Serialize)]
struct Request<'a> {
    dir: &'a str,
    text: &'a str,
}

#[derive(Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: String,
}

pub struct Mt {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Mt {
    pub fn spawn(python: &Path, script: &Path, threads: u32) -> Result<Self> {
        let mut child = Command::new(python)
            .arg(script)
            .arg(threads.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("could not launch {}", python.display()))?;

        let stdin = child.stdin.take().context("sidecar has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("sidecar has no stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn translate(&mut self, dir: &str, text: &str) -> Result<String> {
        let req = serde_json::to_string(&Request { dir, text })?;
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;

        let mut line = String::new();
        let n = self.stdout.read_line(&mut line)?;
        if n == 0 {
            bail!("the MT sidecar closed the connection");
        }
        let resp: Response = serde_json::from_str(line.trim())?;
        if !resp.ok {
            bail!("MT failed: {}", resp.error);
        }
        Ok(resp.text)
    }
}

impl Drop for Mt {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
