# HTTP Downloader 设计文档

**日期**: 2026-01-26
**版本**: 1.0
**状态**: 设计完成，待实现

## 概述

实现一个高性能的 HTTP 下载器，支持分片并发下载、断点续传、缓存管理和 SHA256 校验。使用多线程架构，每个分片由独立线程处理下载和文件写入。

## 技术栈

- **HTTP 客户端**: reqwest (blocking API)
- **并发模型**: 标准线程 (`std::thread`)
- **序列化**: serde + serde_json
- **哈希计算**: sha2
- **文件 I/O**: 标准 std::fs（不使用 Direct I/O）

## 架构设计

### 核心组件

#### 1. DownloadManager（下载管理器）
- 整体协调下载任务
- 管理缓存查找和校验
- 负责状态持久化和恢复
- 启动和等待工作线程

#### 2. ChunkDownloader（分片下载器）
- 每个工作线程运行一个分片下载器
- 使用 reqwest blocking API 发送 HTTP Range 请求
- 接收响应流并写入临时文件
- 完成后更新共享状态

#### 3. StatePersistence（状态持久化）
- JSON 格式保存下载状态
- 记录每个分片的完成状态
- 支持断点续传

#### 4. FileMerger（文件合并器）
- 所有分片下载完成后按顺序合并
- 边合并边计算 SHA256
- 校验通过后移动到目标位置

#### 5. CacheManager（缓存管理器）
- 基于 URL + SHA256 的缓存机制
- 缓存命中时直接复制文件
- 缓存失效时重新下载

## 详细设计

### 1. 分片策略

#### 文件大小分类

- **小文件** (< 16MB): 不分片，单线程下载
- **中等文件** (16MB - 128MB): 2-4 个分片
- **大文件** (> 128MB): 根据 CPU 核心数，最多 16 个分片

#### 分片计算逻辑

```rust
struct ChunkingConfig {
    min_chunk_size: u64,      // 最小分片大小: 5MB
    max_chunks: usize,        // 最大分片数: 16
    small_file_threshold: u64, // 小文件阈值: 16MB
    medium_file_threshold: u64, // 中等文件阈值: 128MB
}

fn calculate_chunks(file_size: u64, config: &ChunkingConfig) -> usize {
    if file_size < config.small_file_threshold {
        // 小文件：不分片
        return 1;
    } else if file_size < config.medium_file_threshold {
        // 中等文件：2-4 个分片
        let chunks = (file_size / config.min_chunk_size).max(2).min(4);
        return chunks as usize;
    } else {
        // 大文件：根据文件大小和最小分片大小计算，不超过 max_chunks
        let chunks = file_size / config.min_chunk_size;
        return chunks.min(config.max_chunks as u64) as usize;
    }
}
```

#### Range 请求计算

```
分片 0: bytes=0-{chunk_size-1}
分片 1: bytes={chunk_size}-{chunk_size*2-1}
...
分片 N: bytes={start}-{file_size-1}  // 最后一个分片到文件末尾
```

### 2. 线程架构

#### 主线程职责

1. 协调整体流程
2. 创建状态文件（包含所有分片信息）
3. 生成并启动工作线程
4. 等待所有线程完成
5. 执行文件合并和 SHA256 校验
6. 清理状态文件和临时文件

#### 工作线程职责

每个工作线程负责一个完整的分片：

1. 使用 reqwest blocking API 发送 HTTP Range 请求
2. 接收响应流
3. 边下载边写入临时文件 `{url_hash}.part{N}`
4. 完成后更新共享状态（需要加锁 `Arc<Mutex<DownloadState>>`）
5. 失败时标记状态为 Failed 并重试（最多 3 次）

#### 线程架构图

```
[主线程]
  ├─ 创建状态文件 (所有分片标记为 Pending)
  ├─ 启动 N 个工作线程
  │   ├─ [线程 0] → HTTP Range 下载分片 0 → 写入 .part0 → 更新状态
  │   ├─ [线程 1] → HTTP Range 下载分片 1 → 写入 .part1 → 更新状态
  │   ├─ [线程 2] → HTTP Range 下载分片 2 → 写入 .part2 → 更新状态
  │   └─ ...
  ├─ 等待所有线程完成 (join)
  ├─ 合并所有 .part 文件（边合并边计算 SHA256）
  ├─ SHA256 校验
  └─ 清理状态文件和临时文件
```

### 3. 状态持久化和断点续传

#### 状态文件结构

**路径**: `{temp_dir}/{url_hash}.state.json`

```rust
struct DownloadState {
    url: String,
    expected_sha256: String,
    file_size: u64,
    total_chunks: usize,
    chunk_size: u64,
    chunks: Vec<ChunkState>,
    created_at: i64,
    updated_at: i64,
}

struct ChunkState {
    index: usize,
    start: u64,
    end: u64,
    status: ChunkStatus,
    temp_file: PathBuf,  // {url_hash}.part{index}
}

enum ChunkStatus {
    Pending,
    Completed,
    Failed,
}
```

#### 状态文件生命周期

1. **主逻辑创建**（启动下载前）
   - 检查缓存
   - 发送 HEAD 请求获取文件大小
   - 计算分片策略
   - 创建完整的状态文件（所有分片标记为 Pending）

2. **工作线程更新**（下载过程中）
   - 每个线程完成后更新自己的分片状态：Pending → Completed
   - 失败时标记为 Failed
   - 需要获取 `Arc<Mutex<DownloadState>>` 锁
   - 更新后立即持久化到磁盘（原子性写入：先写临时文件，再 rename）

3. **主逻辑清理**（最后）
   - 等待所有线程完成
   - 合并临时文件（可能失败）
   - SHA256 校验（可能失败）
   - 只有全部成功后才删除状态文件和临时文件
   - 任何步骤失败都保留状态，下次可以恢复

#### 断点续传流程

**启动时检查：**

1. 计算 URL 的 SHA256 哈希值
2. 检查是否存在状态文件 `{url_hash}.state.json`
3. 如果存在，读取并验证：
   - URL 匹配
   - expected_sha256 匹配
   - file_size 匹配
4. 检查每个已完成分片的临时文件是否存在且大小正确
5. 标记缺失或不完整的分片为 Pending，重新下载
6. 只启动需要下载的分片线程

**断点续传粒度：**
- 分片级别：已完成的分片不重新下载，未完成的分片从头开始
- 简单可靠，浪费的带宽有限

### 4. 缓存机制

#### 缓存目录结构

```
cache_dir/
  ├── {url_hash_1}/
  │   ├── content          # 实际文件内容
  │   └── metadata.json    # 元数据
  ├── {url_hash_2}/
  │   ├── content
  │   └── metadata.json
  ...
```

#### metadata.json 结构

```rust
struct CacheMetadata {
    url: String,
    sha256: String,
    file_size: u64,
    downloaded_at: i64,
}
```

#### 缓存查找流程

1. **计算 URL 哈希**
   - 使用 SHA256(URL) 的前 16 字节（32 个十六进制字符）作为目录名

2. **检查缓存目录是否存在**
   - 不存在 → 执行下载

3. **读取 metadata.json**
   - 验证 URL 匹配（避免哈希碰撞）
   - 获取存储的 SHA256

4. **验证文件完整性**
   - 检查 `content` 文件是否存在
   - 计算文件的 SHA256
   - 与 expected_sha256 比较

5. **缓存命中**
   - SHA256 匹配 → 复制到目标位置
   - SHA256 不匹配 → 删除缓存，重新下载

#### 缓存写入

下载成功并校验通过后：
1. 创建缓存目录 `{cache_dir}/{url_hash}/`
2. 复制最终文件到 `content`
3. 写入 `metadata.json`

### 5. 文件合并和校验

#### 合并策略：流式顺序合并

```rust
use sha2::{Sha256, Digest};

fn merge_and_verify(
    chunks: &[ChunkState],
    target_path: &Path,
    expected_sha256: &str,
) -> Result<(), DownloadError> {
    let mut hasher = Sha256::new();
    let mut target_file = File::create(target_path)?;

    for chunk in chunks {
        let mut part_file = File::open(&chunk.temp_file)?;
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

        loop {
            let n = part_file.read(&mut buffer)?;
            if n == 0 { break; }

            hasher.update(&buffer[..n]);  // 更新哈希
            target_file.write_all(&buffer[..n])?; // 写入目标文件
        }
    }

    let computed_hash = format!("{:x}", hasher.finalize());
    if computed_hash != expected_sha256 {
        fs::remove_file(target_path)?;
        return Err(DownloadError::Sha256Mismatch {
            expected: expected_sha256.to_string(),
            actual: computed_hash,
        });
    }

    Ok(())
}
```

**优点：**
- 只读取一次分片文件
- 边合并边计算哈希，节省时间
- 内存占用固定（只有 64KB 缓冲区）

#### 成功后清理

1. 删除所有 `.part{N}` 文件
2. 删除状态文件 `.state.json`
3. 写入缓存（如果启用）

#### 失败处理

- 任何分片文件缺失或读取失败 → 整个合并失败
- 保留状态文件和剩余的临时文件
- 返回错误，下次可以恢复

### 6. 错误处理和重试机制

#### 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("HTTP error {status} for URL: {url}")]
    HttpError { status: u16, url: String },

    #[error("Server does not support Range requests")]
    RangeNotSupported,

    #[error("File write error: {0}")]
    FileWriteError(#[source] std::io::Error),

    #[error("File read error: {0}")]
    FileReadError(#[source] std::io::Error),

    #[error("SHA256 mismatch: expected {expected}, got {actual}")]
    Sha256Mismatch { expected: String, actual: String },

    #[error("Download state corrupted")]
    StateCorrupted,

    #[error("Chunk {index} is missing")]
    ChunkMissing { index: usize },
}
```

#### 重试策略

**分片级别重试（工作线程内部）**
- 网络临时错误（超时、连接重置）：最多重试 3 次
- 指数退避：1s, 2s, 4s
- HTTP 4xx 错误：不重试，直接失败
- HTTP 5xx 错误：重试
- 每次重试前在状态文件中记录失败次数

**整体下载重试**
- 由调用方决定是否重新启动整个下载
- 状态文件保留，支持断点续传

#### Range 支持检测

**启动前检测：**

1. 发送 HEAD 请求，检查响应头：
   - `Accept-Ranges: bytes` → 支持 Range
   - `Content-Length` → 获取文件大小

2. 如果不支持 Range：
   - 小文件（< 16MB）：降级为单线程完整下载
   - 大文件：返回错误或警告

**降级策略：**
- 不支持 Range 时自动切换为单线程完整下载
- 不创建分片，直接下载到临时文件
- 完成后仍然进行 SHA256 校验

#### 线程 Panic 处理

- 使用 `std::thread::Builder` 设置 panic handler
- 任何线程 panic 应该被捕获并转换为 Error
- 主线程通过 `JoinHandle.join()` 检测线程失败
- 标记对应分片为 Failed，保留状态文件

### 7. 公共 API 设计

#### 核心类型

```rust
pub struct Downloader {
    config: DownloaderConfig,
}

pub struct DownloaderConfig {
    // 分片配置
    pub min_chunk_size: u64,        // 默认 5MB
    pub max_chunks: usize,          // 默认 16
    pub small_file_threshold: u64,  // 默认 16MB
    pub medium_file_threshold: u64, // 默认 128MB

    // 缓存配置
    pub cache_dir: Option<PathBuf>,

    // 临时文件目录
    pub temp_dir: PathBuf,  // 默认系统临时目录

    // 网络配置
    pub timeout: Duration,          // 默认 30s
    pub max_retries: usize,         // 默认 3
    pub user_agent: Option<String>,
}

pub struct DownloadRequest {
    pub url: String,
    pub expected_sha256: String,
    pub output_path: PathBuf,
}

pub struct DownloadResult {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub from_cache: bool,
    pub duration: Duration,
}
```

#### 主要方法

```rust
impl Downloader {
    /// 创建新的下载器实例
    pub fn new(config: DownloaderConfig) -> Self;

    /// 执行下载任务
    /// - 检查缓存
    /// - 如果缓存未命中，执行分片下载
    /// - 合并分片并校验 SHA256
    /// - 写入缓存（如果启用）
    pub fn download(&self, request: DownloadRequest) -> Result<DownloadResult, DownloadError>;

    /// 检查并恢复未完成的下载
    /// - 读取状态文件
    /// - 只下载未完成的分片
    pub fn resume(&self, url: &str) -> Result<DownloadResult, DownloadError>;

    /// 清理指定 URL 的所有临时文件和状态
    pub fn cleanup(&self, url: &str) -> Result<(), DownloadError>;
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        Self {
            min_chunk_size: 5 * 1024 * 1024,      // 5MB
            max_chunks: 16,
            small_file_threshold: 16 * 1024 * 1024,   // 16MB
            medium_file_threshold: 128 * 1024 * 1024, // 128MB
            cache_dir: None,
            temp_dir: std::env::temp_dir(),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            user_agent: None,
        }
    }
}
```

#### 使用示例

```rust
use downloader::{Downloader, DownloaderConfig, DownloadRequest};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DownloaderConfig {
        cache_dir: Some(PathBuf::from("/path/to/cache")),
        temp_dir: PathBuf::from("/tmp/downloads"),
        ..Default::default()
    };

    let downloader = Downloader::new(config);

    let request = DownloadRequest {
        url: "https://example.com/large-file.zip".to_string(),
        expected_sha256: "abc123def456...".to_string(),
        output_path: PathBuf::from("/path/to/output.zip"),
    };

    match downloader.download(request) {
        Ok(result) => {
            println!("Downloaded {} bytes in {:?}", result.size, result.duration);
            println!("SHA256: {}", result.sha256);
            println!("From cache: {}", result.from_cache);
        }
        Err(e) => {
            eprintln!("Download failed: {:?}", e);
            // 可以稍后调用 resume 重试
        }
    }

    Ok(())
}
```

#### 可选扩展：进度回调

```rust
pub struct DownloadProgress {
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub completed_chunks: usize,
    pub total_chunks: usize,
}

pub type ProgressCallback = Box<dyn Fn(DownloadProgress) + Send + Sync>;

// 在 DownloadRequest 中添加
pub struct DownloadRequest {
    pub url: String,
    pub expected_sha256: String,
    pub output_path: PathBuf,
    pub progress_callback: Option<ProgressCallback>,
}
```

进度回调通过共享状态 `Arc<Mutex<DownloadState>>` 实现：
- 每个线程下载数据后更新已下载字节数
- 主线程定期读取状态并调用回调

## 实现顺序

### Phase 1: 基础功能
1. 基本的 `DownloadRequest` 和 `DownloaderConfig` 结构
2. 单线程下载（不分片）
3. 文件写入和 SHA256 校验
4. 基础错误处理

### Phase 2: 分片下载
1. 文件大小检测和分片计算
2. Range 请求支持检测
3. 多线程分片下载
4. 临时文件管理
5. 文件合并

### Phase 3: 状态持久化
1. 状态文件的序列化和反序列化
2. 断点续传逻辑
3. 状态更新的线程安全

### Phase 4: 缓存机制
1. 缓存目录结构
2. 缓存查找和验证
3. 缓存写入

### Phase 5: 增强功能
1. 重试机制和指数退避
2. 进度回调（可选）
3. 更完善的错误处理
4. 日志和监控

## 依赖项

```toml
[dependencies]
reqwest = { version = "0.11", features = ["blocking"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha2 = "0.10"
thiserror = "1.0"
```

## 测试策略

### 单元测试
- 分片计算逻辑
- 状态文件序列化/反序列化
- SHA256 计算
- 缓存查找逻辑

### 集成测试
- 完整的下载流程
- 断点续传
- 缓存命中/未命中
- Range 支持检测和降级
- 各种网络错误场景

### 性能测试
- 不同文件大小的下载性能
- 不同分片数量的影响
- 内存占用
- 磁盘 I/O 性能

## 未来优化方向

1. **Direct I/O 支持**
   - Linux 上使用 `O_DIRECT`
   - macOS 上使用 `F_NOCACHE`
   - 需要处理对齐要求

2. **更智能的分片策略**
   - 根据网络速度动态调整
   - 根据磁盘性能调整

3. **HTTP/2 多路复用**
   - 使用 reqwest 的 async API
   - 单个连接多个请求

4. **压缩传输**
   - 支持 gzip/brotli
   - 边下载边解压

5. **更细粒度的断点续传**
   - 字节级别的续传
   - 减少重复下载

## 总结

这个设计提供了一个功能完整、性能良好、易于使用的 HTTP 下载器。核心特性包括：

- ✅ 智能分片并发下载
- ✅ 基于分片级别的断点续传
- ✅ URL + SHA256 缓存机制
- ✅ 完整的错误处理和重试
- ✅ 简单的多线程架构
- ✅ 灵活的配置选项
- ✅ 清晰的公共 API

设计优先考虑可靠性和简单性，避免过度工程化，后续可以根据实际需求逐步添加优化。
