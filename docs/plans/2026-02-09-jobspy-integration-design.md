# JobSpy Integration Design

## Goal

Integrate [JobSpy](https://github.com/speedyapply/JobSpy) (Python job scraping library) into the Rust job platform via PyO3, enabling real job discovery from 8+ job boards (LinkedIn, Indeed, Glassdoor, Google, ZipRecruiter, Bayt, Naukri, BDJobs).

## Architecture

### Crate Layout

```
crates/
  jobspy-sys/                    # New: PyO3 wrapper for python-jobspy
    Cargo.toml
    build.rs                     # Python env setup via ytmusicapi-uv
    src/
      lib.rs                     # JobSpy struct + scrape_jobs()
      setup.rs                   # OnceLock runtime initialization
      types.rs                   # ScrapedJob, SiteName, JobType enums

  domain/job-source/
    src/
      jobspy.rs                  # New: JobSpyDriver impl JobSourceDriver
```

### Data Flow

```
DiscoveryCriteria
  → JobSpyDriver.fetch_jobs()
    → jobspy_sys::JobSpy::scrape_jobs(params)
      → Python GIL: jobspy.scrape_jobs() → DataFrame
      → df.to_json(orient="records") → JSON string
      → serde_json::from_str() → Vec<ScrapedJob>
    → ScrapedJob → RawJob
  → JobSpyDriver.normalize()
    → RawJob → NormalizedJob (validate title+company, trim, generate UUID)
```

### Key Dependencies

- `pyo3 = "0.27"` with `auto-initialize` feature
- `pyo3-build-config = "0.27"` (build dep)
- `ytmusicapi-uv` via git dep from `https://github.com/crrow/ytmusicapi`
- `serde`, `serde_json` for data interchange
- `dirs` for cache directory

## jobspy-sys Crate Design

### types.rs — Rust Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SiteName {
    LinkedIn,
    Indeed,
    Glassdoor,
    Google,
    ZipRecruiter,
    Bayt,
    Naukri,
    BDJobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobType {
    FullTime,
    PartTime,
    Internship,
    Contract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapeParams {
    pub site_name: Vec<SiteName>,
    pub search_term: String,
    pub location: Option<String>,
    pub distance: Option<u32>,
    pub job_type: Option<JobType>,
    pub is_remote: Option<bool>,
    pub results_wanted: Option<u32>,
    pub hours_old: Option<u32>,
    pub easy_apply: Option<bool>,
    pub country_indeed: Option<String>,
    pub linkedin_fetch_description: Option<bool>,
    pub enforce_annual_salary: Option<bool>,
    pub proxies: Option<Vec<String>>,
    pub verbose: Option<u32>,
}

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
    // Platform-specific
    pub job_level: Option<String>,        // LinkedIn
    pub company_industry: Option<String>, // LinkedIn
    pub emails: Option<Vec<String>>,
}
```

### lib.rs — Public API

```rust
pub struct JobSpy { /* private */ }

impl JobSpy {
    pub fn new() -> Result<Self, JobSpyError> { /* init Python env, import jobspy */ }
    pub fn scrape_jobs(&self, params: &ScrapeParams) -> Result<Vec<ScrapedJob>, JobSpyError> { /* call Python */ }
}
```

### build.rs — Python Setup

Following ytmusicapi-sys pattern:
1. Use `UvManager` to ensure Python 3.10
2. Create venv at `~/.cache/jobspy/venv-py3.10`
3. `pip install python-jobspy`
4. Set `PYO3_PYTHON` env var
5. Configure linker search paths
6. Save `build-config.json`

### setup.rs — Runtime Init

- `OnceLock<RuntimeConfig>` for one-time init
- Load build-config.json
- Set `PYTHONHOME`, `sys.path` to include site-packages
- Verify `import jobspy` succeeds

## JobSpyDriver Design

```rust
pub struct JobSpyDriver {
    jobspy: jobspy_sys::JobSpy,
    default_sites: Vec<SiteName>,
}

impl JobSourceDriver for JobSpyDriver {
    fn source_name(&self) -> &str { "jobspy" }

    async fn fetch_jobs(&self, criteria: &DiscoveryCriteria) -> Result<Vec<RawJob>, SourceError> {
        // Map DiscoveryCriteria → ScrapeParams
        // Call jobspy.scrape_jobs()
        // Map Vec<ScrapedJob> → Vec<RawJob>
    }

    async fn normalize(&self, raw: RawJob) -> Result<NormalizedJob, SourceError> {
        // Validate title + company required
        // Trim whitespace
        // Generate UUID
    }
}
```

### Feature Flag

job-source crate uses feature flag `jobspy`:
```toml
[features]
jobspy = ["dep:jobspy-sys"]

[dependencies]
jobspy-sys = { workspace = true, optional = true }
```

## Issues

1. **#52: Create `jobspy-sys` crate** — PyO3 wrapper, build.rs, types, tests
2. **#53: Implement `JobSpyDriver`** — Domain integration, trait impl, mapping
3. **#54: App integration** — Composition root, config, feature flag
