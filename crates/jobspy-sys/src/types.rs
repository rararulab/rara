use serde::{Deserialize, Serialize};

/// Supported job board sites for scraping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SiteName {
    LinkedIn,
    Indeed,
    Glassdoor,
    Google,
    #[serde(rename = "zip_recruiter", alias = "ziprecruiter")]
    ZipRecruiter,
    Bayt,
    Naukri,
    #[serde(rename = "bdjobs")]
    BDJobs,
}

impl SiteName {
    /// Returns the Python-compatible string representation used by
    /// `python-jobspy`.
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
    #[serde(rename = "fulltime", alias = "full-time", alias = "full_time")]
    FullTime,
    #[serde(rename = "parttime", alias = "part-time", alias = "part_time")]
    PartTime,
    #[serde(alias = "intern")]
    Internship,
    #[serde(alias = "contractor")]
    Contract,
}

impl JobType {
    /// Returns the Python-compatible string representation used by
    /// `python-jobspy`.
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
///
/// Use the builder to construct — only `site_name` and `search_term` are
/// required, all other fields default to `None`:
///
/// ```ignore
/// let params = ScrapeParams::builder()
///     .site_name(vec![SiteName::Indeed, SiteName::LinkedIn])
///     .search_term("rust developer".to_owned())
///     .results_wanted(20)
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct ScrapeParams {
    pub site_name:    Vec<SiteName>,
    pub search_term:  String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location:     Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance:     Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_type:     Option<JobType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_remote:    Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_wanted: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_old:    Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub easy_apply:   Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_indeed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkedin_fetch_description: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_annual_salary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxies:      Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbose:      Option<u32>,
}

/// A single job result from `JobSpy`'s `scrape_jobs()` `DataFrame`.
///
/// All fields are optional because different sites return different data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapedJob {
    pub site:             Option<String>,
    pub title:            Option<String>,
    pub company:          Option<String>,
    pub company_url:      Option<String>,
    pub job_url:          Option<String>,
    pub city:             Option<String>,
    pub state:            Option<String>,
    pub country:          Option<String>,
    pub job_type:         Option<String>,
    pub is_remote:        Option<bool>,
    pub description:      Option<String>,
    pub date_posted:      Option<String>,
    pub min_amount:       Option<f64>,
    pub max_amount:       Option<f64>,
    pub currency:         Option<String>,
    pub salary_source:    Option<String>,
    pub salary_interval:  Option<String>,
    pub job_level:        Option<String>,
    pub company_industry: Option<String>,
    pub emails:           Option<Vec<String>>,
}
