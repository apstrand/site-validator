use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub source: String,
    pub line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub url: String,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub response_time_ms: f64,
    pub occurrences: Vec<Occurrence>,
}

impl CheckResult {
    pub fn is_broken(&self) -> bool {
        self.error.is_some() || self.status_code.map_or(false, |s| s >= 400)
    }

    pub fn is_redirect(&self) -> bool {
        self.error.is_none() && self.status_code.map_or(false, |s| (300..400).contains(&s))
    }

    pub fn status_label(&self) -> String {
        if let Some(ref e) = self.error {
            e.chars().take(60).collect()
        } else {
            self.status_code.map_or_else(|| "?".to_string(), |c| c.to_string())
        }
    }
}

pub struct ValidationReport {
    pub target: String,
    pub mode: &'static str,
    pub pages_crawled: usize,
    pub results: Vec<CheckResult>,
    pub duration_secs: f64,
}

impl ValidationReport {
    pub fn broken(&self) -> Vec<&CheckResult> {
        self.results.iter().filter(|r| r.is_broken()).collect()
    }

    pub fn redirects(&self) -> Vec<&CheckResult> {
        self.results.iter().filter(|r| r.is_redirect()).collect()
    }

    pub fn is_success(&self) -> bool {
        !self.results.iter().any(|r| r.is_broken())
    }
}
