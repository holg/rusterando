use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kitchen_protocol::SeqId;
use serde::{Deserialize, Serialize};

const MAX_REMEMBERED: usize = 1000;
const FILE_NAME: &str = "idempotency.json";

/// Persists "what have we already printed?" across restarts. After a reboot
/// or reconnect, the server will replay anything past `last_acked_seq`; we
/// dedupe against `recent` to make absolutely sure we don't double-print a
/// receipt that was already fired before we crashed.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct IdempotencyCache {
    #[serde(skip)]
    path: PathBuf,
    last_acked: Option<SeqId>,
    recent: VecDeque<SeqId>,
}

impl IdempotencyCache {
    pub fn load(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir).context("create state dir")?;
        let path = state_dir.join(FILE_NAME);
        let mut cache: Self = if path.exists() {
            let text = std::fs::read_to_string(&path).context("read idempotency file")?;
            // Corrupt file → start fresh; the worst case is we reprint one or two
            // orders the server's outbox still has unacked. Better than refusing
            // to start.
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            Self::default()
        };
        cache.path = path;
        Ok(cache)
    }

    pub fn contains(&self, seq_id: SeqId) -> bool {
        self.recent.iter().any(|s| *s == seq_id)
    }

    pub fn last_acked_seq(&self) -> Option<SeqId> {
        self.last_acked
    }

    pub fn record(&mut self, seq_id: SeqId) -> Result<()> {
        if !self.contains(seq_id) {
            self.recent.push_back(seq_id);
            while self.recent.len() > MAX_REMEMBERED {
                self.recent.pop_front();
            }
        }
        self.last_acked = Some(match self.last_acked {
            Some(s) if s.0 >= seq_id.0 => s,
            _ => seq_id,
        });
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let text = serde_json::to_string(self).context("encode idempotency")?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, text).context("write idempotency tmp")?;
        // Atomic swap so a crash mid-write doesn't corrupt the live file.
        std::fs::rename(&tmp, &self.path).context("rename idempotency tmp")?;
        Ok(())
    }
}
