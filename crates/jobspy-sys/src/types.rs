// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use serde::{Deserialize, Deserializer, Serialize};

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
    pub site_name:                  Vec<SiteName>,
    pub search_term:                String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location:                   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance:                   Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_type:                   Option<JobType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_remote:                  Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_wanted:             Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_old:                  Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub easy_apply:                 Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_indeed:             Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkedin_fetch_description: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce_annual_salary:      Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxies:                    Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbose:                    Option<u32>,
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
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub emails:           Option<Vec<String>>,
}

/// Deserialize a field that pandas may serialize as either a single string
/// or an array of strings (or null).
fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    struct StringOrVec;

    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Option<Vec<String>>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("null, a string, or a list of strings")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(vec![v.to_owned()]))
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut v = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                v.push(s);
            }
            Ok(if v.is_empty() { None } else { Some(v) })
        }
    }

    deserializer.deserialize_any(StringOrVec)
}
