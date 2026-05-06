use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Found,
    Missing,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub label: String,
    pub status: Status,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn has_missing(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Missing)
    }

    pub fn print(&self) {
        for c in &self.checks {
            println!("  [{:?}] {}", c.status, c.label);
            if let Some(r) = &c.remediation {
                println!("    → {r}");
            }
        }
    }
}

pub async fn check_all(_config: &Config) -> Report {
    todo!("phase 9: rewrite prereq checks against the new schema")
}
