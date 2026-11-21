//! Watch for known vulnerabilities (CVE, RustSec, GitHub advisories) in a project's
//! dependencies, with each ecosystem's own audit tool, run by the person's own install:
//!
//! - Rust: `cargo audit --json` (cargo-audit) when the folder has a Cargo.lock;
//! - JavaScript: `npm audit --json` when it has a package-lock.json.
//!
//! Orchestra itself sends nothing: the tools fetch their advisory databases (cargo-audit) or
//! send the dependency tree to the npm registry (npm audit), and only when the person asks.

use serde_json::Value;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eco {
    Cargo,
    Npm,
}

impl Eco {
    pub fn label(self) -> &'static str {
        match self {
            Eco::Cargo => "Rust (Cargo.lock)",
            Eco::Npm => "npm (package-lock.json)",
        }
    }

    fn lock_file(self) -> &'static str {
        match self {
            Eco::Cargo => "Cargo.lock",
            Eco::Npm => "package-lock.json",
        }
    }

    /// The program that must be there for the audit to run.
    fn tool(self) -> &'static str {
        match self {
            Eco::Cargo => "cargo-audit",
            Eco::Npm => "npm",
        }
    }

    /// How to install the tool, when it's missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Eco::Cargo => "cargo install cargo-audit",
            Eco::Npm => "https://nodejs.org",
        }
    }
}

/// How bad a finding is, from the advisory's own rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Unknown,
    Low,
    Moderate,
    High,
    Critical,
}

impl Severity {
    fn parse(s: &str) -> Severity {
        match s.to_ascii_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "moderate" | "medium" => Severity::Moderate,
            "low" | "info" => Severity::Low,
            _ => Severity::Unknown,
        }
    }

    /// From a CVSS v3 base score.
    fn of_score(score: f64) -> Severity {
        if score >= 9.0 {
            Severity::Critical
        } else if score >= 7.0 {
            Severity::High
        } else if score >= 4.0 {
            Severity::Moderate
        } else if score > 0.0 {
            Severity::Low
        } else {
            Severity::Unknown
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => crate::tr!("critique", "critical"),
            Severity::High => crate::tr!("élevée", "high"),
            Severity::Moderate => crate::tr!("moyenne", "moderate"),
            Severity::Low => crate::tr!("faible", "low"),
            Severity::Unknown => crate::tr!("non notée", "unrated"),
        }
    }

    /// Important enough to fix right away (high or critical).
    pub fn important(self) -> bool {
        self >= Severity::High
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// CVE id when the advisory has one, else its own (RUSTSEC-…, GHSA-…).
    pub id: String,
    pub package: String,
    pub version: String,
    pub title: String,
    pub severity: Severity,
    /// The versions that fix it ("≥ 1.2.3"), when known.
    pub fix: Option<String>,
    pub url: Option<String>,
}

/// One ecosystem's result: its findings, or why it couldn't be checked.
#[derive(Clone, Debug, PartialEq)]
pub struct EcoReport {
    pub eco: Eco,
    pub result: Result<Vec<Finding>, String>,
}

/// The ecosystems whose lock file sits at the root of `folder`.
pub fn ecosystems(folder: &Path) -> Vec<Eco> {
    [Eco::Cargo, Eco::Npm]
        .into_iter()
        .filter(|e| folder.join(e.lock_file()).is_file())
        .collect()
}

/// Audit every ecosystem of `folder` (runs the tools: call it off the UI thread).
pub fn check(folder: &Path) -> Vec<EcoReport> {
    let path = crate::sync::shell_path();
    ecosystems(folder)
        .into_iter()
        .map(|eco| EcoReport { eco, result: check_one(eco, folder, &path) })
        .collect()
}

fn check_one(eco: Eco, folder: &Path, path: &str) -> Result<Vec<Finding>, String> {
    if crate::sync::which(eco.tool(), path).is_none() {
        return Err(crate::trf!(
            "{} n'est pas installé ({})",
            "{} isn't installed ({})",
            eco.tool(),
            eco.install_hint()
        ));
    }
    let (prog, args): (&str, &[&str]) = match eco {
        Eco::Cargo => ("cargo", &["audit", "--json"]),
        Eco::Npm => ("npm", &["audit", "--json"]),
    };
    let out = crate::proc::command(prog)
        .args(args)
        .current_dir(folder)
        .env("PATH", path)
        .output()
        .map_err(|e| e.to_string())?;
    // Both tools exit with an error code when they find something: the JSON tells.
    let text = String::from_utf8_lossy(&out.stdout);
    let parsed = match eco {
        Eco::Cargo => parse_cargo(&text),
        Eco::Npm => parse_npm(&text),
    };
    parsed.ok_or_else(|| {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        crate::trf!(
            "l'audit n'a pas abouti{}",
            "the audit didn't complete{}",
            if last.is_empty() { String::new() } else { format!(" : {}", last.trim()) }
        )
    })
}

fn sorted(mut v: Vec<Finding>) -> Vec<Finding> {
    v.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.package.cmp(&b.package)));
    v
}

/// `cargo audit --json`'s output; None when it isn't one.
pub fn parse_cargo(text: &str) -> Option<Vec<Finding>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let list = v.get("vulnerabilities")?.get("list")?.as_array()?;
    let out = list
        .iter()
        .filter_map(|x| {
            let adv = x.get("advisory")?;
            let own = adv.get("id")?.as_str()?.to_string();
            let cve = adv
                .get("aliases")
                .and_then(Value::as_array)
                .and_then(|a| a.iter().filter_map(Value::as_str).find(|s| s.starts_with("CVE-")))
                .map(str::to_string);
            let patched: Vec<&str> = x
                .pointer("/versions/patched")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let severity = adv
                .get("severity")
                .and_then(Value::as_str)
                .map(Severity::parse)
                .or_else(|| adv.get("cvss").and_then(Value::as_str).and_then(cvss_score).map(Severity::of_score))
                .unwrap_or(Severity::Unknown);
            Some(Finding {
                url: adv
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                    .map(str::to_string)
                    .or_else(|| Some(format!("https://rustsec.org/advisories/{own}.html"))),
                id: cve.unwrap_or(own),
                package: x.pointer("/package/name").and_then(Value::as_str).unwrap_or("?").to_string(),
                version: x.pointer("/package/version").and_then(Value::as_str).unwrap_or("").to_string(),
                title: adv.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                severity,
                fix: (!patched.is_empty()).then(|| patched.join(", ")),
            })
        })
        .collect();
    Some(sorted(out))
}

/// A CVSS v3 vector's base score ("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" → 9.8).
fn cvss_score(vector: &str) -> Option<f64> {
    let mut m = std::collections::HashMap::new();
    for part in vector.split('/').skip(1) {
        let (k, v) = part.split_once(':')?;
        m.insert(k, v);
    }
    let changed = *m.get("S")? == "C";
    let av = match *m.get("AV")? { "N" => 0.85, "A" => 0.62, "L" => 0.55, _ => 0.2 };
    let ac = match *m.get("AC")? { "L" => 0.77, _ => 0.44 };
    let pr = match (*m.get("PR")?, changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        (_, false) => 0.27,
        (_, true) => 0.5,
    };
    let ui = match *m.get("UI")? { "N" => 0.85, _ => 0.62 };
    let cia = |k: &str| -> Option<f64> {
        Some(match *m.get(k)? { "H" => 0.56, "L" => 0.22, _ => 0.0 })
    };
    let iss = 1.0 - (1.0 - cia("C")?) * (1.0 - cia("I")?) * (1.0 - cia("A")?);
    let impact = if changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15)
    } else {
        6.42 * iss
    };
    if impact <= 0.0 {
        return Some(0.0);
    }
    let expl = 8.22 * av * ac * pr * ui;
    let raw = if changed { (1.08 * (impact + expl)).min(10.0) } else { (impact + expl).min(10.0) };
    // Rounded up to one decimal, as the standard says.
    Some((raw * 10.0 - 1e-9).ceil() / 10.0)
}

/// `npm audit --json`'s output (npm 7 and later); None when it isn't one.
pub fn parse_npm(text: &str) -> Option<Vec<Finding>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let vulns = v.get("vulnerabilities")?.as_object()?;
    let mut out = vec![];
    for (name, x) in vulns {
        // Only the advisories themselves: a package flagged through another one ("via": a
        // name) is listed under that one.
        let Some(via) = x.get("via").and_then(Value::as_array) else { continue };
        let fix = match x.get("fixAvailable") {
            Some(Value::Object(o)) => o
                .get("version")
                .and_then(Value::as_str)
                .map(|ver| format!("{} {ver}", o.get("name").and_then(Value::as_str).unwrap_or(name))),
            Some(Value::Bool(true)) => Some("npm audit fix".to_string()),
            _ => None,
        };
        for a in via.iter().filter(|a| a.is_object()) {
            let url = a.get("url").and_then(Value::as_str).map(str::to_string);
            let id = url
                .as_deref()
                .and_then(|u| u.rsplit('/').next())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| a.get("source").map(|s| s.to_string()))
                .unwrap_or_else(|| "?".into());
            if out.iter().any(|f: &Finding| f.id == id && f.package == *name) {
                continue;
            }
            out.push(Finding {
                id,
                package: a.get("name").and_then(Value::as_str).unwrap_or(name).to_string(),
                version: a.get("range").and_then(Value::as_str).unwrap_or("").to_string(),
                title: a.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                severity: a
                    .get("severity")
                    .and_then(Value::as_str)
                    .map(Severity::parse)
                    .unwrap_or(Severity::Unknown),
                fix: fix.clone(),
                url,
            });
        }
    }
    Some(sorted(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_audit_findings_are_read() {
        let json = r#"{"database":{},"lockfile":{"dependency-count":120},
          "vulnerabilities":{"found":true,"count":2,"list":[
            {"advisory":{"id":"RUSTSEC-2024-0001","package":"foo","title":"Out-of-bounds read",
               "aliases":["GHSA-xxxx","CVE-2024-1234"],"cvss":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H","url":""},
             "versions":{"patched":[">=1.2.3"],"unaffected":[]},
             "package":{"name":"foo","version":"1.2.0"}},
            {"advisory":{"id":"RUSTSEC-2023-0099","package":"bar","title":"Unsound","aliases":[],"cvss":null},
             "versions":{"patched":[]},
             "package":{"name":"bar","version":"0.3.1"}}]},
          "warnings":{}}"#;
        let f = parse_cargo(json).unwrap();
        assert_eq!(f.len(), 2);
        // The most serious first; the CVE id when there is one.
        assert_eq!(f[0].id, "CVE-2024-1234");
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].severity.important());
        assert_eq!(f[0].fix.as_deref(), Some(">=1.2.3"));
        assert_eq!((f[0].package.as_str(), f[0].version.as_str()), ("foo", "1.2.0"));
        assert_eq!(f[0].url.as_deref(), Some("https://rustsec.org/advisories/RUSTSEC-2024-0001.html"));
        assert_eq!(f[1].id, "RUSTSEC-2023-0099");
        assert_eq!(f[1].severity, Severity::Unknown);
        assert_eq!(f[1].fix, None);
        // Nothing found, and output that isn't the audit's.
        let clean = r#"{"vulnerabilities":{"found":false,"count":0,"list":[]}}"#;
        assert_eq!(parse_cargo(clean), Some(vec![]));
        assert_eq!(parse_cargo("error: not found"), None);
    }

    #[test]
    fn cvss_vectors_give_their_base_score() {
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"), Some(9.8));
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H"), Some(7.5));
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:L/I:N/A:N"), Some(5.3));
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"), Some(10.0));
        assert_eq!(cvss_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N"), Some(0.0));
        assert_eq!(cvss_score("nonsense"), None);
    }

    #[test]
    fn npm_audit_findings_are_read() {
        let json = r#"{"auditReportVersion":2,"vulnerabilities":{
          "lodash":{"name":"lodash","severity":"high","via":[
             {"source":1096305,"name":"lodash","title":"Prototype Pollution","url":"https://github.com/advisories/GHSA-p6mc-m468-83gw","severity":"high","range":"<4.17.19"}],
             "fixAvailable":true},
          "app-dep":{"name":"app-dep","severity":"high","via":["lodash"],"fixAvailable":{"name":"app-dep","version":"2.0.0"}},
          "minimist":{"name":"minimist","severity":"critical","via":[
             {"source":1,"name":"minimist","title":"Prototype Pollution","url":"https://github.com/advisories/GHSA-xvch-5gv4-984h","severity":"critical","range":"<1.2.6"}],
             "fixAvailable":{"name":"mkdirp","version":"1.0.4"}}},
          "metadata":{"vulnerabilities":{"total":3}}}"#;
        let f = parse_npm(json).unwrap();
        // Only the advisories, not the packages flagged through them; the worst first.
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].id, "GHSA-xvch-5gv4-984h");
        assert_eq!(f[0].severity, Severity::Critical);
        assert_eq!(f[0].fix.as_deref(), Some("mkdirp 1.0.4"));
        assert_eq!(f[1].package, "lodash");
        assert_eq!(f[1].fix.as_deref(), Some("npm audit fix"));
        assert_eq!(f[1].version, "<4.17.19");
        assert_eq!(parse_npm(r#"{"vulnerabilities":{}}"#), Some(vec![]));
        assert_eq!(parse_npm(""), None);
    }

    #[test]
    fn ecosystems_come_from_the_lock_files() {
        let dir = std::env::temp_dir().join(format!("orch-audit-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(ecosystems(&dir).is_empty());
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        assert_eq!(ecosystems(&dir), [Eco::Npm]);
        std::fs::write(dir.join("Cargo.lock"), "").unwrap();
        assert_eq!(ecosystems(&dir), [Eco::Cargo, Eco::Npm]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
