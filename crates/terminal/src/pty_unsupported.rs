//! Hors Windows : pas de pseudo-console (le projet ne cible que Windows).

use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::TerminalError;

#[derive(Debug, Clone)]
pub(crate) struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub columns: u16,
    pub lines: u16,
}

pub(crate) struct Pty;
pub(crate) struct PtyReader;

impl PtyReader {
    pub fn read(&mut self, _buffer: &mut [u8]) -> Result<usize, TerminalError> {
        Ok(0)
    }
}

impl Pty {
    pub fn spawn(_spec: &SpawnSpec) -> Result<(Pty, PtyReader), TerminalError> {
        Err(TerminalError::Unsupported)
    }
    pub fn pid(&self) -> u32 {
        0
    }
    pub fn write_all(&self, _bytes: &[u8]) -> Result<(), TerminalError> {
        Ok(())
    }
    pub fn resize(&self, _columns: u16, _lines: u16) -> Result<(), TerminalError> {
        Ok(())
    }
    pub fn try_wait(&self) -> Option<u32> {
        Some(0)
    }
    pub fn wait(&self, _timeout_ms: u32) -> Option<u32> {
        Some(0)
    }
    pub fn kill_tree(&self) {}
    pub fn shutdown(&self) {}
    pub fn process_ids(&self) -> Vec<u32> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

pub(crate) fn process_snapshot() -> Vec<ProcessEntry> {
    Vec::new()
}
