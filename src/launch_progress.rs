//! Events emitted by a browser CLI process for the native launch dialog.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchStage {
    CheckProxy,
    ClosePeers,
    SwitchIp,
    GeoIp,
    VerifyGeo,
    StartBrowser,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchEvent {
    Stage {
        stage: LaunchStage,
        detail: Option<String>,
    },
    Ready,
    Failed {
        message: String,
    },
}

pub struct LaunchProgressWriter(Option<PathBuf>);

impl LaunchProgressWriter {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self(path)
    }

    pub fn emit(&self, event: LaunchEvent) -> Result<()> {
        let Some(path) = &self.0 else {
            return Ok(());
        };
        append_event(path, &event)
    }

    pub fn stage(&self, stage: LaunchStage, detail: Option<String>) -> Result<()> {
        self.emit(LaunchEvent::Stage { stage, detail })
    }
}

fn append_event(path: &Path, event: &LaunchEvent) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

pub fn read_events(path: &Path) -> Result<Vec<LaunchEvent>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    content
        .lines()
        .take(if content.ends_with('\n') {
            usize::MAX
        } else {
            content.lines().count().saturating_sub(1)
        })
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_stages_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("open.progress");
        let writer = LaunchProgressWriter::new(Some(path.clone()));
        writer.stage(LaunchStage::CheckProxy, None).unwrap();
        writer
            .stage(LaunchStage::ClosePeers, Some("2".into()))
            .unwrap();
        writer.emit(LaunchEvent::Ready).unwrap();
        assert_eq!(read_events(&path).unwrap().len(), 3);
        assert!(matches!(read_events(&path).unwrap()[2], LaunchEvent::Ready));
    }
}
