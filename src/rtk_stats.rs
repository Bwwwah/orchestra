//! What RTK saved in a session, read from its database (`Agent::rtk_db`, table `commands`).
//!
//! A shortened output does not save its tokens once: it stays in the conversation, and the
//! model is sent it again with every later request. So each saving counts as many times as
//! the output would have been sent back (`repeated_savings`).

use std::path::Path;

/// One row of RTK's `commands` table: when the command ran (ms since 1970) and the tokens
/// its shortened output saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saving {
    pub at_ms: u64,
    pub saved: u64,
}

/// The savings recorded in the database `db` (read only; none if it does not exist yet or
/// cannot be read). Rows with a negative saving (RTK's output was longer) count for nothing.
pub fn read_savings(db: &Path) -> Vec<Saving> {
    use rusqlite::{Connection, OpenFlags};
    let Ok(conn) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT timestamp, saved_tokens FROM commands") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else {
        return Vec::new();
    };
    rows.flatten()
        .filter(|(_, saved)| *saved > 0)
        .filter_map(|(ts, saved)| {
            Some(Saving { at_ms: crate::sessions::parse_iso_ms(&ts)?, saved: saved as u64 })
        })
        .collect()
}

/// The tokens saved, each saving counted once for the request that received the shortened
/// output and once more for every later request of the session (`requests_ms`: when the
/// model was called, in ms).
pub fn repeated_savings(savings: &[Saving], requests_ms: &[u64]) -> u64 {
    savings
        .iter()
        .map(|s| {
            let later = requests_ms.iter().filter(|&&t| t > s.at_ms).count() as u64;
            s.saved * (1 + later)
        })
        .sum()
}

/// When the model was called in the conversation file `path` (ms since 1970, sorted): one
/// per distinct Claude Code assistant message, one per Codex `token_count` event.
pub fn request_times(path: &Path, tool: crate::sessions::Tool) -> Vec<u64> {
    use crate::sessions::Tool;
    use serde_json::Value;
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<u64> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            let at = crate::sessions::parse_iso_ms(v.get("timestamp")?.as_str()?)?;
            match tool {
                Tool::ClaudeCode => {
                    if v.get("type")?.as_str()? != "assistant" {
                        return None;
                    }
                    let id = v.get("message")?.get("id")?.as_str()?;
                    seen.insert(id.to_string()).then_some(at)
                }
                Tool::Codex => {
                    let p = v.get("payload")?;
                    (p.get("type")?.as_str()? == "token_count").then_some(at)
                }
            }
        })
        .collect();
    out.sort_unstable();
    out
}

/// What RTK saved over a week for one AI and what that AI consumed, in tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WeekSaving {
    pub saved: u64,
    pub consumed: u64,
    /// Copies of the week that ran with RTK (a database with savings).
    pub rtk_sessions: u32,
}

impl WeekSaving {
    /// The share of consumption saved: saved / (consumed + saved), in percent. None without
    /// any token.
    pub fn percent(&self) -> Option<f64> {
        let total = self.saved + self.consumed;
        (total > 0).then(|| 100.0 * self.saved as f64 / total as f64)
    }
}

/// The rolling 7 days up to `now_ms` for the sessions of `tool`, run by `agent` in the copies
/// `folders`: the savings of each copy's RTK database (counted for every later request of
/// the copy's sessions), against the consumption of the sessions of the week.
pub fn week_saving(
    sessions: &[crate::sessions::SessionInfo],
    tool: crate::sessions::Tool,
    agent: crate::auto::Agent,
    folders: &[std::path::PathBuf],
    now_ms: u64,
) -> WeekSaving {
    let since = now_ms.saturating_sub(7 * 24 * 3600 * 1000);
    let mut w = WeekSaving::default();
    for folder in folders {
        let savings: Vec<Saving> =
            read_savings(&agent.rtk_db(folder)).into_iter().filter(|s| s.at_ms >= since).collect();
        let mut requests = Vec::new();
        for s in sessions.iter().filter(|s| {
            s.tool == tool && s.last_activity >= since && Path::new(&s.cwd).starts_with(folder)
        }) {
            requests.extend(request_times(&s.path, tool).into_iter().filter(|t| *t >= since));
        }
        if !savings.is_empty() {
            w.rtk_sessions += 1;
        }
        w.saved += repeated_savings(&savings, &requests);
        w.consumed += crate::sessions::consumption(sessions, tool, folder, since).billed;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_of_a_week() {
        assert_eq!(WeekSaving::default().percent(), None);
        let w = WeekSaving { saved: 25, consumed: 75, rtk_sessions: 1 };
        assert_eq!(w.percent(), Some(25.0));
    }

    #[test]
    fn a_saving_counts_for_every_later_request() {
        let savings = [Saving { at_ms: 100, saved: 10 }, Saving { at_ms: 300, saved: 5 }];
        // Requests at 50 (before both), 200 (after the first), 400 (after both).
        assert_eq!(repeated_savings(&savings, &[50, 200, 400]), 10 * 3 + 5 * 2);
        assert_eq!(repeated_savings(&savings, &[]), 15);
        assert_eq!(repeated_savings(&[], &[1, 2]), 0);
    }

    #[test]
    fn reads_the_commands_table_of_a_database() {
        let path = std::env::temp_dir().join(format!("orchestra-rtk-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE commands (id INTEGER PRIMARY KEY, timestamp TEXT NOT NULL,
                 original_cmd TEXT NOT NULL, rtk_cmd TEXT NOT NULL, input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL, saved_tokens INTEGER NOT NULL, savings_pct REAL NOT NULL);
                 INSERT INTO commands VALUES (1,'1970-01-01T00:00:01Z','git status','rtk git status',100,20,80,80.0);
                 INSERT INTO commands VALUES (2,'1970-01-01T00:00:02Z','ls','rtk ls',10,20,-10,0.0);",
            )
            .unwrap();
        }
        assert_eq!(read_savings(&path), vec![Saving { at_ms: 1000, saved: 80 }]);
        let _ = std::fs::remove_file(&path);
        assert!(read_savings(&path).is_empty());
    }
}
