# 持久化队列设计文档

## 概述

为交易撮合场景设计的低延迟、高吞吐持久化队列，基于 mmap 实现，灵感来源于 Chronicle Queue。

## 设计目标

- **延迟目标**: 10-100μs（几十微秒级）
- **并发模式**: 初期支持 SPSC，后续扩展到 MPSC/SPMC/MPMC
- **持久化**: 默认异步刷盘，可配置为同步或批量刷盘
- **吞吐量**: 百万级 TPS
- **可靠性**: CRC 校验 + 崩溃恢复

## 架构设计

### 整体架构

```
[Appender] --> [Crossbeam Channel] --> [IO Worker Thread] --> [mmap file]
   ^                                          |
   |                                          v
 用户线程                                  专用 I/O 线程
 (纳秒级)                              (异步刷盘，微秒级)
```

### 关键特性

1. **Append-Only**: mmap 文件只追加，不随机写入
2. **零拷贝**: 使用 `bytes::Bytes` 避免数据拷贝
3. **无锁设计**: Atomic 操作 + 内存屏障保证并发安全
4. **异步 I/O**: 用户线程只写内存，后台线程负责刷盘
5. **灵活滚动**: 支持按大小、时间、消息数滚动文件

## 文件组织

### 目录结构（按时间分层）

```
/path/to/queue/
├── metadata.json          # 队列全局元数据
├── .lock                  # 可选的进程锁文件
├── 2026/                  # 年份目录
│   └── 01/                # 月份目录
│       └── 14/            # 日期目录
│           ├── 20260114-0001.data
│           ├── 20260114-0001.index
│           ├── 20260114-0002.data
│           └── 20260114-0002.index
```

### 文件命名

- **时间戳前缀**: `YYYYMMDD-序号`
- **扩展名**: `.data` 和 `.index`
- **示例**: `20260114-0001.data`

### 数据文件格式 (.data)

预分配固定大小，零填充，消息紧凑排列：

```
[Header: 4KB - 预留]
[Message 1: length(4B) + payload(变长) + CRC64(8B)]
[Message 2: length(4B) + payload(变长) + CRC64(8B)]
...
[Zeros: 未使用空间]
```

消息格式：
- **Length**: 4 字节（消息体长度，不含 length 和 CRC）
- **Payload**: 变长数据
- **CRC64**: 8 字节（覆盖 length + payload）

### 索引文件格式 (.index)

稀疏索引，每 N 条消息（默认 1024）记录一个索引点：

```
[Header: 256B - 包含索引间隔、消息总数等]
[Entry 0: sequence(8B) + offset(8B)]
[Entry 1: sequence(8B) + offset(8B)]
...
```

索引项：
- **Sequence**: 8 字节（全局递增的消息序号）
- **Offset**: 8 字节（在 .data 文件中的字节偏移）

## 核心数据结构

### 配置

```rust
pub struct QueueConfig {
    pub base_path: PathBuf,
    pub file_size: u64,              // 数据文件预分配大小（默认 1GB）
    pub roll_strategy: RollStrategy, // 文件滚动策略
    pub flush_mode: FlushMode,       // 刷盘模式
    pub index_interval: u64,         // 索引间隔（默认 1024）
    pub verify_on_startup: bool,     // 启动时是否验证 CRC
}

pub enum RollStrategy {
    BySize(u64),                     // 按文件大小滚动
    ByTime(Duration),                // 按时间周期滚动
    ByCount(u64),                    // 按消息数量滚动
    Combined(Vec<RollStrategy>),     // 组合策略
}

pub enum FlushMode {
    Async,                           // 异步刷盘（依赖 OS）
    Sync,                            // 每条消息同步刷盘
    Batch { bytes: usize, interval: Duration }, // 批量刷盘
}
```

### 核心类型

```rust
// 队列实例
pub struct Queue {
    config: Arc<QueueConfig>,
    io_tx: Sender<WriteEvent>,
    worker_manager: Arc<Manager>,
    global_sequence: Arc<AtomicU64>,
}

// 写入者
pub struct Appender {
    tx: Sender<WriteEvent>,
    sequence: Arc<AtomicU64>,
}

// 读取者
pub struct Tailer {
    config: Arc<QueueConfig>,
    current_file: Option<ReadOnlyDataFile>,
    current_sequence: u64,
    read_position: usize,
}

// 写入事件
struct WriteEvent {
    sequence: u64,
    data: Bytes,        // 使用 bytes::Bytes 零拷贝
    timestamp: u64,
}

// 消息引用
pub struct Message<'a> {
    pub sequence: u64,
    pub timestamp: u64,
    pub payload: &'a [u8],  // 零拷贝，指向 mmap 区域
}
```

### DataFile (基于 mmap-io)

```rust
use mmap_io::{MemoryMappedFile, MmapMode, MmapAdvice};

pub struct DataFile {
    file: File,
    mmap: MemoryMappedFile,
    path: PathBuf,
    size: u64,
}

impl DataFile {
    // 创建新文件（预分配 + 零填充）
    pub fn create(path: PathBuf, size: u64) -> Result<Self>;

    // 打开已存在的文件
    pub fn open(path: PathBuf) -> Result<Self>;

    // 获取可写指针
    pub unsafe fn as_mut_ptr(&self) -> *mut u8;

    // 刷盘
    pub fn flush(&self, mode: FlushMode) -> Result<()>;

    // 部分刷盘
    pub fn flush_range(&self, offset: usize, len: usize) -> Result<()>;
}
```

## 并发控制

### SPSC 模式（单写入者）

```rust
impl Appender {
    pub fn append(&self, data: impl Into<Bytes>) -> Result<u64> {
        // 1. 分配序号（Relaxed：单写入者无竞争）
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);

        // 2. 发送到 IO 线程（crossbeam channel）
        self.tx.send(WriteEvent {
            sequence: seq,
            data: data.into(),
            timestamp: timestamp_micros(),
        })?;

        Ok(seq)
    }
}
```

### IO Worker Thread（同步任务）

```rust
pub struct IOWorker {
    rx: Receiver<WriteEvent>,
    current_file: Option<DataFile>,
    write_position: usize,
    index_writer: IndexWriter,
    config: Arc<QueueConfig>,
    pending_bytes: usize,
    last_flush: Instant,
}

impl BlockingWorker for IOWorker {
    fn work<S: Clone + Send + Sync>(&mut self, ctx: WorkerContext<S>) {
        // 阻塞接收（带超时）
        match self.rx.recv_timeout(Duration::from_micros(100)) {
            Ok(event) => {
                self.write_event(event);
            }
            Err(RecvTimeoutError::Timeout) => {
                self.check_flush();
            }
            Err(RecvTimeoutError::Disconnected) => {
                ctx.request_shutdown();
            }
        }
    }
}

fn write_event(&mut self, event: WriteEvent) -> Result<()> {
    let total_size = 4 + event.data.len() + 8;

    // 检查文件滚动
    if self.write_position + total_size > self.config.file_size {
        self.roll_file()?;
    }

    let pos = self.write_position;

    unsafe {
        let ptr = file.as_mut_ptr().add(pos);

        // Length
        ptr.cast::<u32>().write_unaligned(event.data.len() as u32);

        // Payload
        std::ptr::copy_nonoverlapping(
            event.data.as_ptr(),
            ptr.add(4),
            event.data.len()
        );

        // CRC
        let crc = calculate_crc(&event.data);
        ptr.add(4 + event.data.len()).cast::<u64>().write_unaligned(crc);
    }

    self.write_position += total_size;
    self.pending_bytes += total_size;

    // 根据配置刷盘
    self.handle_flush(&event)?;

    Ok(())
}
```

### 无锁读取

```rust
impl Tailer {
    pub fn read_next(&mut self) -> Result<Option<Message>> {
        let read_pos = self.read_position;

        // 读取 length（零填充检测）
        let length = unsafe {
            self.current_file.as_ptr().add(read_pos).cast::<u32>().read()
        };

        if length == 0 {
            // 遇到零填充，切换到下一个文件
            return self.switch_to_next_file();
        }

        // 读取 payload
        let payload = unsafe {
            std::slice::from_raw_parts(
                self.current_file.as_ptr().add(read_pos + 4),
                length as usize
            )
        };

        // 读取并验证 CRC
        let crc = unsafe {
            self.current_file.as_ptr()
                .add(read_pos + 4 + length as usize)
                .cast::<u64>()
                .read()
        };

        if calculate_crc(payload) != crc {
            return Err(Error::CorruptedMessage(self.current_sequence));
        }

        // 更新读取位置
        self.read_position += 4 + length as usize + 8;
        self.current_sequence += 1;

        Ok(Some(Message {
            sequence: self.current_sequence - 1,
            timestamp: 0, // TODO: 从文件读取
            payload,
        }))
    }
}
```

## 崩溃恢复

### 启动时恢复流程

1. **扫描目录结构**: 按时间顺序找到所有 `.data` 和 `.index` 文件
2. **定位最后一个文件**: 找到最新的数据文件
3. **扫描验证**: 从头扫描，通过零填充检测找到最后一条完整消息
4. **CRC 校验**: 如果配置了 `verify_on_startup`，验证所有消息的 CRC
5. **恢复写入位置**: 设置 `write_position` 和 `global_sequence`

### 损坏消息处理

- **预分配 + 零填充**: 文件预分配并填零，通过检测零字节判断有效数据结尾
- **CRC64 校验**: 每条消息带 CRC，读取时验证
- **跳过损坏消息**: 遇到 CRC 失败的消息可选择跳过或报错

## API 设计

### Builder API

```rust
let queue = QueueBuilder::new("/data/trading_queue")
    .file_size(1024 * 1024 * 1024)  // 1GB per file
    .roll_strategy(RollStrategy::Combined(vec![
        RollStrategy::BySize(1024 * 1024 * 1024),
        RollStrategy::ByTime(Duration::from_hours(1)),
    ]))
    .flush_mode(FlushMode::Batch {
        bytes: 4096,
        interval: Duration::from_millis(10),
    })
    .index_interval(1024)
    .verify_on_startup(false)
    .build()?;
```

### 写入 API

```rust
// 创建写入者
let appender = queue.create_appender();

// 单条写入
let seq = appender.append(b"order data")?;

// 使用 Bytes（零拷贝）
let data = Bytes::from("order data");
let seq = appender.append(data)?;

// 批量写入
let orders = vec![
    Bytes::from("order1"),
    Bytes::from("order2"),
];
let sequences = appender.append_batch(orders)?;
```

### 读取 API

```rust
// 从头开始读取
let mut tailer = queue.create_tailer()?;

// 从指定位置读取
let mut tailer = queue.create_tailer_at(100)?;

// 读取消息
while let Some(msg) = tailer.read_next()? {
    println!("Seq: {}, Payload: {:?}", msg.sequence, msg.payload);
}

// 随机访问
tailer.seek(500)?;

// 迭代器模式
for result in tailer.into_iter() {
    let msg = result?;
    // 处理消息
}
```

### 生命周期管理

```rust
// 优雅关闭
queue.shutdown().await?;
```

## 扩展计划

### MPSC 支持（多写入者）

```rust
impl Appender {
    pub fn append_mpsc(&self, data: impl Into<Bytes>) -> Result<u64> {
        // CAS 操作预留空间
        let seq = self.sequence.fetch_add(1, Ordering::AcqRel);

        // 发送到 IO 线程
        self.tx.send(WriteEvent {
            sequence: seq,
            data: data.into(),
            timestamp: timestamp_micros(),
        })?;

        Ok(seq)
    }
}
```

### SPMC 支持（多读取者）

每个 Tailer 维护独立的读取位置，通过不同的 sequence 起点隔离。

### 类型系统表达

```rust
pub struct Queue<M: Mode> {
    _mode: PhantomData<M>,
    // ...
}

pub trait Mode: sealed::Sealed {}
pub struct SPSC;
pub struct MPSC;
pub struct SPMC;
```

## 性能优化

### 写入路径优化

1. **用户线程**: 只做序号分配 + channel 发送（几十纳秒）
2. **IO 线程**: 批量写入 + 定时刷盘
3. **零拷贝**: `bytes::Bytes` 引用计数，避免数据拷贝
4. **预分配**: 文件预分配，避免动态扩展

### 读取路径优化

1. **零拷贝**: `Message` 持有 mmap 区域的引用
2. **稀疏索引**: 快速定位，O(log N) 查找
3. **顺序访问**: mmap 配合 `MmapAdvice::Sequential` 优化
4. **预读**: `MmapAdvice::WillNeed` 提示 OS 预读

### 内存优化

- **mmap**: 利用操作系统页缓存，减少内存拷贝
- **预分配**: 避免频繁的文件扩展系统调用

## 依赖

```toml
[dependencies]
mmap-io = "0.9"
crossbeam = "0.8"
bytes = "1.0"
# 使用现有的 worker 抽象（需要扩展支持 BlockingWorker）
rsketch-common-worker = { path = "../worker" }
```

## 后续工作

1. **扩展 Worker 抽象**: 添加 `BlockingWorker` trait 支持同步任务
2. **实现核心结构**: `Queue`, `Appender`, `Tailer`, `IOWorker`
3. **实现文件操作**: `DataFile`, `IndexWriter`, `IndexReader`
4. **实现崩溃恢复**: 启动时扫描和验证逻辑
5. **性能测试**: 延迟、吞吐量基准测试
6. **扩展并发模式**: MPSC/SPMC/MPMC 支持

## 参考

- [Chronicle Queue](https://github.com/OpenHFT/Chronicle-Queue) - Java 实现的低延迟持久化队列
- [mmap-io](https://crates.io/crates/mmap-io) - Rust mmap 库
- [crossbeam](https://crates.io/crates/crossbeam) - Rust 并发原语库
- [bytes](https://crates.io/crates/bytes) - Rust 零拷贝字节处理库
