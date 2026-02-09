use serde::{Deserialize, Serialize};

/// Supported job board sites for scraping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SiteName {
    LinkedIn,
    Indeed,
    Glassdoor,
    Google,
    #[serde(rename = "zip_recruiter")]
    ZipRecruiter,
    Bayt,
    Naukri,
    #[serde(rename = "bdjobs")]
    BDJobs,
}

impl SiteName {
    /// Returns the Python-compatible string representation used by `python-jobspy`.
    #[must_use]
    pub const fn as_python_str(&self) -> &'static str {
        match self {
            Self::LinkedIn => "linkedin",
            Self::Indeed => "indeed",
            Self::Glassdoor => "glassdoor",
            Self::Google => "google",
            Self::ZipRecruiter => "zip_recruiter",
            Self::Bayt => "bayt",
            Self::Naukri => "naukri",
            Self::BDJobs => "bdjobs",
        }
    }
}

/// Job employment type filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    #[serde(rename = "fulltime")]
    FullTime,
    #[serde(rename = "parttime")]
    PartTime,
    Internship,
    Contract,
}

impl JobType {
    /// Returns the Python-compatible string representation used by `python-jobspy`.
    #[must_use]
    pub const fn as_python_str(&self) -> &'static str {
        match self {
            Self::FullTime => "fulltime",
            Self::PartTime => "parttime",
            Self::Internship => "internship",
            Self::Contract => "contract",
        }
    }
}

/// Parameters for a `JobSpy` scrape operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeParams {
    pub site_name: Vec<SiteName>,
    pub search_term: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_type: Option<JobType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_remote: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_wanted: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_old: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub easy_apply: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_indeed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkedin_fetch_description: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_annual_salary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbose: Option<u32>,
}

/// A single job result from `JobSpy`'s `scrape_jobs()` `DataFrame`.
///
/// All fields are optional because different sites return different data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedJob {
    pub site: Option<String>,
    pub title: Option<String>,
    pub company: Option<String>,
    pub company_url: Option<String>,
    pub job_url: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub job_type: Option<String>,
    pub is_remote: Option<bool>,
    pub description: Option<String>,
    pub date_posted: Option<String>,
    pub min_amount: Option<f64>,
    pub max_amount: Option<f64>,
    pub currency: Option<String>,
    pub salary_source: Option<String>,
    pub salary_interval: Option<String>,
    pub job_level: Option<String>,
    pub company_industry: Option<String>,
    pub emails: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_name_python_str() {
        assert_eq!(SiteName::LinkedIn.as_python_str(), "linkedin");
        assert_eq!(SiteName::ZipRecruiter.as_python_str(), "zip_recruiter");
        assert_eq!(SiteName::BDJobs.as_python_str(), "bdjobs");
    }

    #[test]
    fn job_type_python_str() {
        assert_eq!(JobType::FullTime.as_python_str(), "fulltime");
        assert_eq!(JobType::PartTime.as_python_str(), "parttime");
        assert_eq!(JobType::Internship.as_python_str(), "internship");
        assert_eq!(JobType::Contract.as_python_str(), "contract");
    }

    #[test]
    fn site_name_serde_roundtrip() {
        let sites = vec![
            SiteName::LinkedIn,
            SiteName::Indeed,
            SiteName::ZipRecruiter,
            SiteName::BDJobs,
        ];
        let json = serde_json::to_string(&sites).unwrap();
        let deserialized: Vec<SiteName> = serde_json::from_str(&json).unwrap();
        assert_eq!(sites, deserialized);
    }

    #[test]
    fn job_type_serde_roundtrip() {
        let types = vec![
            JobType::FullTime,
            JobType::PartTime,
            JobType::Internship,
            JobType::Contract,
        ];
        let json = serde_json::to_string(&types).unwrap();
        let deserialized: Vec<JobType> = serde_json::from_str(&json).unwrap();
        assert_eq!(types, deserialized);
    }

    #[test]
    fn scraped_job_deserialize_partial() {
        let json = r#"{"site":"linkedin","title":"Software Engineer","company":null,"job_url":"https://example.com"}"#;
        let job: ScrapedJob = serde_json::from_str(json).unwrap();
        assert_eq!(job.site.as_deref(), Some("linkedin"));
        assert_eq!(job.title.as_deref(), Some("Software Engineer"));
        assert!(job.company.is_none());
        assert_eq!(job.job_url.as_deref(), Some("https://example.com"));
        assert!(job.description.is_none());
    }

    #[test]
    fn scrape_params_serialization() {
        let params = ScrapeParams {
            site_name: vec![SiteName::LinkedIn, SiteName::Indeed],
            search_term: "rust developer".to_string(),
            location: Some("San Francisco".to_string()),
            distance: None,
            job_type: Some(JobType::FullTime),
            is_remote: Some(true),
            results_wanted: Some(10),
            hours_old: None,
            easy_apply: None,
            country_indeed: None,
            linkedin_fetch_description: None,
            enforce_annual_salary: None,
            proxies: None,
            verbose: None,
        };
        let json = serde_json::to_string(&params).unwrap();
        // None fields should be skipped
        assert!(!json.contains("distance"));
        assert!(!json.contains("hours_old"));
        // Present fields should be included
        assert!(json.contains("rust developer"));
        assert!(json.contains("San Francisco"));
    }
}
