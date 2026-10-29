mod python;

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Access {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unresolved {
    pub file: String,
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub files_scanned: usize,
    pub accesses: Vec<Access>,
    pub missing: BTreeMap<String, Vec<Access>>,
    pub unresolved: Vec<Unresolved>,
    pub errors: Vec<String>,
}

pub struct Options {
    pub root: PathBuf,
    pub example: PathBuf,
    pub excludes: Vec<String>,
    pub ignore_keys: BTreeSet<String>,
    pub template: bool,
}

pub fn manifest_names(text: &str) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    // Track quoted values so a line inside a multiline value is never treated as a key.
    let mut quote = None;
    for (i, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if quote.is_some() {
            update_quote(raw, &mut quote);
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line
            .strip_prefix("export")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .unwrap_or(line)
            .trim_start();
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("example line {}: expected NAME=value", i + 1))?;
        let key = key.trim();
        if !valid_name(key) {
            return Err(format!("example line {}: invalid variable name", i + 1));
        }
        names.insert(key.to_string());
        let value = value.trim_start();
        if value.starts_with(['\'', '"']) {
            update_quote(value, &mut quote);
        }
    }
    if quote.is_some() {
        return Err("example has an unterminated quoted value".into());
    }
    Ok(names)
}

fn update_quote(text: &str, quote: &mut Option<char>) {
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if Some(c) == *quote {
            *quote = None;
            // Anything after the closing quote belongs to this value or its comment.
            break;
        } else if quote.is_none() && (c == '\'' || c == '"') {
            *quote = Some(c);
        }
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn scan(options: &Options) -> Report {
    let mut report = Report::default();
    if !options.root.is_dir() {
        report.errors.push("scan path is not a directory".into());
        return report;
    }
    let documented = if options.template {
        BTreeSet::new()
    } else {
        let path = if options.example.is_absolute() {
            options.example.clone()
        } else {
            options.root.join(&options.example)
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match manifest_names(&text) {
                Ok(names) => names,
                Err(error) => {
                    report.errors.push(error);
                    return report;
                }
            },
            Err(error) => {
                report
                    .errors
                    .push(format!("cannot read {}: {error}", path.display()));
                return report;
            }
        }
    };
    let mut overrides = ignore::overrides::OverrideBuilder::new(&options.root);
    for glob in &options.excludes {
        if let Err(error) = overrides.add(&format!("!{glob}")) {
            report.errors.push(format!("invalid exclusion: {error}"));
            return report;
        }
    }
    let overrides = match overrides.build() {
        Ok(overrides) => overrides,
        Err(error) => {
            report.errors.push(error.to_string());
            return report;
        }
    };
    let mut walker = ignore::WalkBuilder::new(&options.root);
    walker
        .require_git(false)
        .follow_links(false)
        .overrides(overrides)
        .sort_by_file_path(Path::cmp);
    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.errors.push(format!("cannot walk source: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file())
            || entry.path().extension().is_none_or(|e| e != "py")
        {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(&options.root).unwrap_or(path);
        let label = relative.to_string_lossy().replace('\\', "/");
        match std::fs::read_to_string(path) {
            Ok(source) => {
                report.files_scanned += 1;
                python::analyze(&source, &label, &mut report);
            }
            Err(error) => report.errors.push(format!("cannot read {label}: {error}")),
        }
    }
    report
        .accesses
        .retain(|a| !options.ignore_keys.contains(&a.name));
    report.accesses.sort_by(|a, b| {
        (&a.name, &a.file, a.line, a.column).cmp(&(&b.name, &b.file, b.line, b.column))
    });
    for access in &report.accesses {
        if !documented.contains(&access.name) {
            report
                .missing
                .entry(access.name.clone())
                .or_default()
                .push(access.clone());
        }
    }
    report
}
