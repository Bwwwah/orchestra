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

#[cfg(test)]
mod tests {
    use super::*;

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
