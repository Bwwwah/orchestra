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

/// The conversation files an agent leaves, when Orchestra reads them (Claude Code, Codex).
pub fn tool_of(agent: crate::auto::Agent) -> Option<crate::sessions::Tool> {
    use crate::auto::Agent;
    match agent {
        Agent::ClaudeCode => Some(crate::sessions::Tool::ClaudeCode),
        Agent::Codex => Some(crate::sessions::Tool::Codex),
        _ => None,
    }
}

/// The week's saving of every agent whose sessions Orchestra reads, over the autonomous
/// copies (`data_dir/worktrees/*`).
pub fn week_savings_all(
    sessions: &[crate::sessions::SessionInfo],
    now_ms: u64,
) -> Vec<(crate::auto::Agent, WeekSaving)> {
    let copies: Vec<std::path::PathBuf> =
        std::fs::read_dir(crate::store::data_dir().join("worktrees"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
    crate::auto::Agent::ALL
        .into_iter()
        .filter_map(|a| Some((a, tool_of(a)?)))
        .map(|(a, tool)| (a, week_saving(sessions, tool, a, &copies, now_ms)))
        .collect()
}

/// Under this share of consumption saved, an AI's tile shows no line.
const MIN_SHOWN_PERCENT: f64 = 1.0;

/// « ≈ 12 % déjà économisé avec RTK (7 jours, estimation) ».
pub fn saving_line(w: &WeekSaving) -> Option<String> {
    // Hidden without any RTK session in the week, and under 1 % (too small to be worth a line).
    let pct = w.percent().filter(|p| w.rtk_sessions > 0 && *p >= MIN_SHOWN_PERCENT)?;
    Some(crate::trf!(
        "≈ {} % déjà économisé avec RTK (7 jours, estimation)",
        "≈ {} % already saved with RTK (7 days, estimate)",
        format!("{pct:.0}")
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_line_names_the_percentage() {
        let w = super::WeekSaving { saved: 12, consumed: 88, rtk_sessions: 1 };
        assert!(super::saving_line(&w).unwrap().contains("12 %"));
        assert!(super::saving_line(&super::WeekSaving::default()).is_none());
    }

    use super::*;

    #[test]
    fn the_line_is_hidden_under_one_percent_or_without_rtk_session() {
        let tiny = WeekSaving { saved: 9, consumed: 991, rtk_sessions: 1 };
        assert!(saving_line(&tiny).is_none());
        let one = WeekSaving { saved: 10, consumed: 990, rtk_sessions: 1 };
        assert!(saving_line(&one).unwrap().contains("1 %"));
        let none = WeekSaving { saved: 50, consumed: 50, rtk_sessions: 0 };
        assert!(saving_line(&none).is_none());
    }

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

    /// A fake RTK database at `db` with the given (timestamp, saved tokens) rows.
    fn fake_db(db: &Path, rows: &[(&str, i64)]) {
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(db);
        let c = rusqlite::Connection::open(db).unwrap();
        c.execute_batch(
            "CREATE TABLE commands (id INTEGER PRIMARY KEY, timestamp TEXT NOT NULL,
             original_cmd TEXT NOT NULL, rtk_cmd TEXT NOT NULL, input_tokens INTEGER NOT NULL,
             output_tokens INTEGER NOT NULL, saved_tokens INTEGER NOT NULL, savings_pct REAL NOT NULL);",
        )
        .unwrap();
        for (ts, saved) in rows {
            c.execute(
                "INSERT INTO commands (timestamp, original_cmd, rtk_cmd, input_tokens, output_tokens, saved_tokens, savings_pct)
                 VALUES (?1, 'git status', 'rtk git status', 0, 0, ?2, 0.0)",
                rusqlite::params![ts, saved],
            )
            .unwrap();
        }
    }

    #[test]
    fn request_times_read_a_fake_conversation_of_each_tool() {
        let dir = std::env::temp_dir().join(format!("orchestra-rtk-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Claude Code: one request per distinct assistant message (m1 is written twice), none
        // for the user's messages; the lines are out of order and one is not JSON.
        let claude = dir.join("claude.jsonl");
        std::fs::write(
            &claude,
            [
                r#"{"type":"assistant","timestamp":"1970-01-01T00:00:20Z","message":{"id":"m2"}}"#,
                r#"{"type":"user","timestamp":"1970-01-01T00:00:05Z","message":{"id":"u1"}}"#,
                r#"{"type":"assistant","timestamp":"1970-01-01T00:00:10Z","message":{"id":"m1"}}"#,
                r#"{"type":"assistant","timestamp":"1970-01-01T00:00:11Z","message":{"id":"m1"}}"#,
                "not json",
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(request_times(&claude, crate::sessions::Tool::ClaudeCode), vec![10_000, 20_000]);
        // Codex: one request per token_count event.
        let codex = dir.join("codex.jsonl");
        std::fs::write(
            &codex,
            [
                r#"{"timestamp":"1970-01-01T00:00:03Z","payload":{"type":"token_count"}}"#,
                r#"{"timestamp":"1970-01-01T00:00:04Z","payload":{"type":"agent_message"}}"#,
                r#"{"timestamp":"1970-01-01T00:00:07Z","payload":{"type":"token_count"}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(request_times(&codex, crate::sessions::Tool::Codex), vec![3_000, 7_000]);
        assert!(request_times(&dir.join("missing.jsonl"), crate::sessions::Tool::Codex).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_week_of_a_fake_conversation_and_a_fake_database() {
        use crate::auto::Agent;
        use crate::sessions::{SessionInfo, Tokens, Tool};
        let root = std::env::temp_dir().join(format!("orchestra-rtk-week-{}", std::process::id()));
        let folder = root.join("copy");
        std::fs::create_dir_all(&root).unwrap();
        let agent = Agent::ClaudeCode;
        let db = agent.rtk_db(&folder);
        // Now is day 10 of 1970: the week starts on day 3. The saving of day 2 is out of it,
        // so is the negative one.
        fake_db(
            &db,
            &[
                ("1970-01-02T00:00:00Z", 1000),
                ("1970-01-06T00:00:00Z", 100),
                ("1970-01-06T00:00:15Z", 10),
                ("1970-01-06T00:00:16Z", -50),
            ],
        );
        let conv = root.join("claude.jsonl");
        std::fs::write(
            &conv,
            [
                r#"{"type":"assistant","timestamp":"1970-01-05T23:59:59Z","message":{"id":"a"}}"#,
                r#"{"type":"assistant","timestamp":"1970-01-06T00:00:10Z","message":{"id":"b"}}"#,
                r#"{"type":"assistant","timestamp":"1970-01-06T00:00:20Z","message":{"id":"c"}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let session = |tool, cwd: &Path, billed| SessionInfo {
            tool,
            id: "s".into(),
            cwd: cwd.display().to_string(),
            title: String::new(),
            last_reply: String::new(),
            model: String::new(),
            git_branch: String::new(),
            started: 0,
            last_activity: 432_000_000 + 20_000,
            user_messages: 1,
            agent_messages: 1,
            tokens: Tokens { billed, cached: 0 },
            path: conv.clone(),
        };
        let sessions = [
            session(Tool::ClaudeCode, &folder, 680),
            // Not counted: another AI's session, and a session outside the copy.
            session(Tool::Codex, &folder, 5000),
            session(Tool::ClaudeCode, &root.join("elsewhere"), 5000),
        ];
        let now = 10 * 24 * 3600 * 1000;
        let w = week_saving(&sessions, Tool::ClaudeCode, agent, std::slice::from_ref(&folder), now);
        // 100 saved at 00:00:00 reaches the 2 requests after it (+ its own): 300; 10 saved at
        // 00:00:15 reaches the request at 00:00:20: 20.
        assert_eq!(w, WeekSaving { saved: 320, consumed: 680, rtk_sessions: 1 });
        assert_eq!(w.percent(), Some(32.0));
        assert!(saving_line(&w).unwrap().contains("32 %"));
        // A copy whose database has nothing in the week has no RTK session.
        let other = root.join("other");
        let none = week_saving(&sessions, Tool::ClaudeCode, agent, &[other], now);
        assert_eq!(none.rtk_sessions, 0);
        assert_eq!(none.saved, 0);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(agent.config_dir(&folder));
    }
}
