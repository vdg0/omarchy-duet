//! TTS sidecar client (daemon/scripts/tts_server.py, based on Piper).
//!
//! Voices are loaded once in the sidecar. We measured that launching the Piper
//! CLI per turn cost ~4.4 s (almost all startup), so this resident process
//! is what makes per-turn latency viable.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Serialize)]
struct Request<'a> {
    voice: &'a str,
    text: &'a str,
    out: String,
}

#[derive(Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    ms: f64,
    #[serde(default)]
    audio_s: f64,
    #[serde(default)]
    error: String,
}

pub struct Tts {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Synthesis {
    /// Time reported by the sidecar (synthesis only, excluding IPC).
    pub ms: f64,
    pub audio_s: f64,
}

impl Tts {
    pub fn spawn(python: &Path, script: &Path) -> Result<Self> {
        let mut child = Command::new(python)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("could not launch {python:?}"))?;

        let stdin = child.stdin.take().context("TTS sidecar has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("TTS sidecar has no stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn synth(&mut self, voice: &str, text: &str, out: &PathBuf) -> Result<Synthesis> {
        let req = serde_json::to_string(&Request {
            voice,
            text,
            out: out.display().to_string(),
        })?;
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;

        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            bail!("the TTS sidecar closed the connection");
        }
        let resp: Response = serde_json::from_str(line.trim())?;
        if !resp.ok {
            bail!("TTS failed: {}", resp.error);
        }
        Ok(Synthesis {
            ms: resp.ms,
            audio_s: resp.audio_s,
        })
    }
}

impl Drop for Tts {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
