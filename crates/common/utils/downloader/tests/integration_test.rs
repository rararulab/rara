// Copyright 2025 Crrow
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

use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::head,
};
use axum_test::TestServer;
use downloader::{ChunkingConfig, DownloadError, DownloadRequest, Downloader, DownloaderConfig};
use rsketch_base::readable_size::ReadableSize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify, oneshot};

#[derive(Clone)]
struct GetHooks {
    started: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    release: Arc<Notify>,
}

#[derive(Clone)]
struct AppState {
    content:       Arc<Vec<u8>>,
    accept_ranges: bool,
    checksum:      Option<String>,
    hooks:         Option<GetHooks>,
}

async fn handle_head(State(state): State<AppState>) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&state.content.len().to_string()).unwrap(),
    );
    if state.accept_ranges {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(checksum) = state.checksum.as_deref() {
        headers.insert(
            HeaderName::from_static("x-checksum-sha256"),
            HeaderValue::from_str(checksum).unwrap(),
        );
    }
    (StatusCode::OK, headers)
}

async fn handle_get(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if let Some(hooks) = state.hooks.as_ref() {
        let mut sender = hooks.started.lock().await;
        if let Some(tx) = sender.take() {
            let _ = tx.send(());
        }
        hooks.release.notified().await;
    }

    let total_len = state.content.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, total_len));

    if state.accept_ranges
        && let Some((start, end)) = range
    {
        let slice = &state.content[start..=end];
        let mut response_headers = HeaderMap::new();
        response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        response_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total_len}")).unwrap(),
        );
        response_headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&slice.len().to_string()).unwrap(),
        );
        return (
            StatusCode::PARTIAL_CONTENT,
            response_headers,
            Bytes::copy_from_slice(slice),
        )
            .into_response();
    }

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&total_len.to_string()).unwrap(),
    );
    (
        StatusCode::OK,
        response_headers,
        Bytes::copy_from_slice(&state.content),
    )
        .into_response()
}

fn parse_range(value: &str, total: usize) -> Option<(usize, usize)> {
    let value = value.strip_prefix("bytes=")?;
    let (start_str, end_str) = value.split_once('-')?;
    let start: usize = start_str.parse().ok()?;
    let end: usize = end_str.parse().ok()?;
    if start <= end && end < total {
        Some((start, end))
    } else {
        None
    }
}

fn create_temp_dir(prefix: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("downloader-{prefix}-"))
        .tempdir()
        .expect("failed to create temp dir")
}

fn create_test_server(state: AppState) -> TestServer {
    let app = Router::new()
        .route("/file", head(handle_head).get(handle_get))
        .with_state(state);

    // Create a test server with HTTP transport for real network access
    TestServer::builder()
        .http_transport()
        .build(app)
        .expect("failed to create test server")
}

/// Get the full URL for the /file endpoint
fn get_file_url(server: &TestServer) -> String {
    let base = server
        .server_address()
        .expect("server should have HTTP address")
        .to_string();
    // server_address() may or may not include a trailing slash, handle both cases
    if base.ends_with('/') {
        format!("{base}file")
    } else {
        format!("{base}/file")
    }
}

#[tokio::test]
async fn download_single_and_cache_hit() {
    let content = b"downloader-cache-test".repeat(512);
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let checksum = Some(format!("{:x}", hasher.finalize()));

    let server = create_test_server(AppState {
        content: Arc::new(content.clone()),
        accept_ranges: false,
        checksum,
        hooks: None,
    });

    let cache_dir = create_temp_dir("cache");
    let temp_dir = create_temp_dir("temp");
    let output_dir = create_temp_dir("out");
    let config = DownloaderConfig {
        cache_dir: cache_dir.path().to_path_buf(),
        temp_dir: temp_dir.path().to_path_buf(),
        ..DownloaderConfig::default()
    };

    let downloader = Downloader::new(config);
    let output_path = output_dir.path().join("file.bin");

    // Test that the server endpoint works first
    let response = server.get("/file").await;
    response.assert_status_ok();

    let url = get_file_url(&server);
    let request = DownloadRequest {
        url:         url.clone(),
        output_path: output_path.clone(),
    };

    let result = downloader.download(request.clone()).await.unwrap();
    assert!(!result.from_cache);
    let file_content = tokio::fs::read(&output_path).await.unwrap();
    assert_eq!(file_content, content);

    let output_path_2 = output_dir.path().join("file-cache.bin");
    let request_2 = DownloadRequest {
        url,
        output_path: output_path_2.clone(),
    };
    let cached = downloader.download(request_2).await.unwrap();
    assert!(cached.from_cache);
    let downloaded_cached = tokio::fs::read(&output_path_2).await.unwrap();
    assert_eq!(downloaded_cached, content);
}

#[tokio::test]
async fn download_parallel_with_range_support() {
    let content = b"parallel-download-test".repeat(1024);
    let server = create_test_server(AppState {
        content:       Arc::new(content.clone()),
        accept_ranges: true,
        checksum:      None,
        hooks:         None,
    });

    let cache_dir = create_temp_dir("cache");
    let temp_dir = create_temp_dir("temp");
    let output_dir = create_temp_dir("out");

    let chunking = ChunkingConfig {
        min_chunk_size:        ReadableSize::kb(1),
        max_chunks:            4,
        small_file_threshold:  ReadableSize(1),
        medium_file_threshold: ReadableSize::mb(1),
    };
    let config = DownloaderConfig {
        cache_dir: cache_dir.path().to_path_buf(),
        temp_dir: temp_dir.path().to_path_buf(),
        chunking,
        ..DownloaderConfig::default()
    };

    let downloader = Downloader::new(config);
    let url = get_file_url(&server);
    let request = DownloadRequest {
        url,
        output_path: output_dir.path().join("parallel.bin"),
    };

    let result = downloader.download(request).await.unwrap();
    assert!(!result.from_cache);
    let file_content = tokio::fs::read(&result.path).await.unwrap();
    assert_eq!(file_content, content);
}

#[tokio::test]
async fn download_fails_when_lock_held() {
    let content = b"lock-test-content".repeat(512);
    let (tx, rx) = oneshot::channel();
    let hooks = GetHooks {
        started: Arc::new(Mutex::new(Some(tx))),
        release: Arc::new(Notify::new()),
    };

    let server = create_test_server(AppState {
        content:       Arc::new(content.clone()),
        accept_ranges: false,
        checksum:      None,
        hooks:         Some(hooks.clone()),
    });

    let cache_dir = create_temp_dir("cache");
    let temp_dir = create_temp_dir("temp");
    let output_dir = create_temp_dir("out");
    let config = DownloaderConfig {
        cache_dir: cache_dir.path().to_path_buf(),
        temp_dir: temp_dir.path().to_path_buf(),
        ..DownloaderConfig::default()
    };

    let downloader = Arc::new(Downloader::new(config));
    let url = get_file_url(&server);
    let request = DownloadRequest {
        url:         url.clone(),
        output_path: output_dir.path().join("locked.bin"),
    };

    let first = {
        let downloader = Arc::clone(&downloader);
        let request = request.clone();
        tokio::spawn(async move { downloader.download(request).await })
    };

    let _ = rx.await;

    let second = downloader
        .download(DownloadRequest {
            url,
            output_path: output_dir.path().join("locked-2.bin"),
        })
        .await;
    assert!(matches!(
        second,
        Err(DownloadError::DownloadInProgress { .. })
    ));

    hooks.release.notify_one();
    let _ = first.await.unwrap();
}

#[tokio::test]
async fn download_resumes_after_interruption() {
    let content = b"resume-after-interruption-test".repeat(2048);

    let server = create_test_server(AppState {
        content:       Arc::new(content.clone()),
        accept_ranges: true,
        checksum:      None,
        hooks:         None,
    });

    let cache_dir = create_temp_dir("cache");
    let temp_dir = create_temp_dir("temp");
    let output_dir = create_temp_dir("out");

    let chunking = ChunkingConfig {
        min_chunk_size:        ReadableSize::kb(1),
        max_chunks:            4,
        small_file_threshold:  ReadableSize(1),
        medium_file_threshold: ReadableSize::mb(1),
    };
    let config = DownloaderConfig {
        cache_dir: cache_dir.path().to_path_buf(),
        temp_dir: temp_dir.path().to_path_buf(),
        chunking,
        max_retries: 3,
        ..DownloaderConfig::default()
    };

    let downloader = Downloader::new(config);
    let url = get_file_url(&server);
    let output_path = output_dir.path().join("resume.bin");

    let request = DownloadRequest {
        url:         url.clone(),
        output_path: output_path.clone(),
    };

    use downloader::{ChunkState, ChunkStatus, DownloadState, calculate_chunk_boundaries};

    let boundaries = calculate_chunk_boundaries(content.len() as u64, 3);
    let temp_dir_path = temp_dir.path();

    // Manually create download state, simulating first chunk completed, others
    // pending
    let url_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    let chunks: Vec<ChunkState> = boundaries
        .iter()
        .enumerate()
        .map(|(index, (start, end))| ChunkState {
            index,
            start: *start,
            end: *end,
            status: if index == 0 {
                ChunkStatus::Completed
            } else {
                ChunkStatus::Pending
            },
            temp_file: temp_dir_path.join(format!("{url_hash}.part{index}")),
            retry_count: 0,
        })
        .collect();

    // Create the first chunk file (simulating already downloaded part)
    let first_chunk = &chunks[0];
    #[allow(clippy::cast_possible_truncation)]
    tokio::fs::write(
        &first_chunk.temp_file,
        &content[first_chunk.start as usize..=first_chunk.end as usize],
    )
    .await
    .expect("failed to write first chunk");

    // Create download state
    let now = jiff::Timestamp::now().as_second();
    let download_state = DownloadState {
        url: url.clone(),
        server_checksum: None,
        file_size: content.len() as u64,
        total_chunks: chunks.len(),
        chunk_size: content.len() as u64 / chunks.len() as u64,
        chunks,
        created_at: now,
        updated_at: now,
    };

    // Save state file
    let state_path = temp_dir_path.join(format!("{url_hash}.state.json"));
    let state_json =
        serde_json::to_string_pretty(&download_state).expect("failed to serialize state");
    tokio::fs::write(&state_path, state_json)
        .await
        .expect("failed to write state file");

    // Second download attempt: should resume from interruption point
    let result = downloader.download(request).await.unwrap();

    // Verify results
    assert!(
        !result.from_cache,
        "should be fresh download, not from cache"
    );
    assert_eq!(result.size, content.len() as u64, "file size should match");

    // Verify downloaded content is correct
    let file_content = tokio::fs::read(&output_path).await.unwrap();
    assert_eq!(
        file_content.len(),
        content.len(),
        "downloaded file size should match"
    );
    assert_eq!(
        file_content, content,
        "downloaded content should match original"
    );

    // Verify state file has been cleaned up
    assert!(
        !tokio::fs::try_exists(&state_path).await.unwrap_or(true),
        "state file should be cleaned up after download completion"
    );
}
