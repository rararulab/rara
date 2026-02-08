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

//! Benchmarks for the persistent queue.
//!
//! Measures:
//! - Single message append latency
//! - Throughput at different message sizes
//! - Batch append performance
//! - Read (tailer) throughput

use std::{hint::black_box, time::Duration};

use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rsketch_common_storage_queue::{FlushMode, Queue, QueueBuilder, RollStrategy};
use tempfile::TempDir;

/// Message sizes to benchmark (bytes)
const MESSAGE_SIZES: &[usize] = &[64, 256, 1024, 4096, 16384];

/// Number of messages for batch/throughput tests
const BATCH_SIZE: usize = 10_000;

/// Create a queue with the given flush mode in a temporary directory
fn create_queue(temp_dir: &TempDir, flush_mode: FlushMode) -> Queue {
    QueueBuilder::new(temp_dir.path())
        .file_size(256 * 1024 * 1024) // 256MB
        .roll_strategy(RollStrategy::BySize(256 * 1024 * 1024))
        .flush_mode(flush_mode)
        .index_interval(1024)
        .build()
        .expect("Failed to create queue")
}

/// Generate a message of the given size
fn generate_message(size: usize) -> Bytes { Bytes::from(vec![0xABu8; size]) }

// =============================================================================
// Single Message Append Latency
// =============================================================================

/// Benchmark single message append latency (Async mode - no fsync)
fn bench_append_latency_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_latency_async");

    for &size in MESSAGE_SIZES {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let queue = create_queue(&temp_dir, FlushMode::Async);
            let appender = queue.create_appender();
            let msg = generate_message(size);

            b.iter(|| {
                appender.append(black_box(msg.clone())).unwrap();
            });

            queue.shutdown().unwrap();
        });
    }

    group.finish();
}

/// Benchmark single message append latency (Sync mode - fsync per write)
fn bench_append_latency_sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_latency_sync");
    // Sync mode is slow, reduce sample size
    group.sample_size(50);

    for &size in &[64, 256, 1024] {
        // Only test smaller sizes for sync mode
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let queue = create_queue(&temp_dir, FlushMode::Sync);
            let appender = queue.create_appender();
            let msg = generate_message(size);

            b.iter(|| {
                appender.append(black_box(msg.clone())).unwrap();
            });

            queue.shutdown().unwrap();
        });
    }

    group.finish();
}

/// Benchmark single message append latency (Batch mode)
fn bench_append_latency_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_latency_batch");

    for &size in MESSAGE_SIZES {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let queue = create_queue(
                &temp_dir,
                FlushMode::Batch {
                    bytes:    64 * 1024, // 64KB
                    interval: Duration::from_millis(10),
                },
            );
            let appender = queue.create_appender();
            let msg = generate_message(size);

            b.iter(|| {
                appender.append(black_box(msg.clone())).unwrap();
            });

            queue.shutdown().unwrap();
        });
    }

    group.finish();
}

// =============================================================================
// Throughput (Messages per Second)
// =============================================================================

/// Benchmark throughput - messages per second (Async mode)
fn bench_throughput_async(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_async");
    group.sample_size(20); // Fewer samples since each iteration writes many messages

    for &size in MESSAGE_SIZES {
        let total_bytes = (size * BATCH_SIZE) as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let queue = create_queue(&temp_dir, FlushMode::Async);
                    let appender = queue.create_appender();
                    let msg = generate_message(size);
                    (temp_dir, queue, appender, msg)
                },
                |(temp_dir, queue, appender, msg)| {
                    for _ in 0..BATCH_SIZE {
                        appender.append(black_box(msg.clone())).unwrap();
                    }
                    queue.shutdown().unwrap();
                    drop(temp_dir);
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

/// Benchmark throughput with Batch flush mode
fn bench_throughput_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput_batch");
    group.sample_size(20);

    for &size in MESSAGE_SIZES {
        let total_bytes = (size * BATCH_SIZE) as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let queue = create_queue(
                        &temp_dir,
                        FlushMode::Batch {
                            bytes:    64 * 1024,
                            interval: Duration::from_millis(10),
                        },
                    );
                    let appender = queue.create_appender();
                    let msg = generate_message(size);
                    (temp_dir, queue, appender, msg)
                },
                |(temp_dir, queue, appender, msg)| {
                    for _ in 0..BATCH_SIZE {
                        appender.append(black_box(msg.clone())).unwrap();
                    }
                    queue.shutdown().unwrap();
                    drop(temp_dir);
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

// =============================================================================
// Batch Append API
// =============================================================================

/// Benchmark the append_batch API vs individual appends
fn bench_append_batch_api(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_batch_api");
    group.sample_size(20);

    let batch_sizes = [100, 1000, 5000];
    let msg_size = 256;

    for &batch_count in &batch_sizes {
        let total_bytes = (msg_size * batch_count) as u64;
        group.throughput(Throughput::Bytes(total_bytes));

        // Individual appends
        group.bench_with_input(
            BenchmarkId::new("individual", batch_count),
            &batch_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let queue = create_queue(&temp_dir, FlushMode::Async);
                        let appender = queue.create_appender();
                        let msg = generate_message(msg_size);
                        (temp_dir, queue, appender, msg, count)
                    },
                    |(temp_dir, queue, appender, msg, count)| {
                        for _ in 0..count {
                            appender.append(black_box(msg.clone())).unwrap();
                        }
                        queue.shutdown().unwrap();
                        drop(temp_dir);
                    },
                    BatchSize::PerIteration,
                );
            },
        );

        // Batch API
        group.bench_with_input(
            BenchmarkId::new("batch_api", batch_count),
            &batch_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let temp_dir = TempDir::new().unwrap();
                        let queue = create_queue(&temp_dir, FlushMode::Async);
                        let appender = queue.create_appender();
                        let msgs: Vec<Bytes> =
                            (0..count).map(|_| generate_message(msg_size)).collect();
                        (temp_dir, queue, appender, msgs)
                    },
                    |(temp_dir, queue, appender, msgs)| {
                        appender.append_batch(black_box(msgs)).unwrap();
                        queue.shutdown().unwrap();
                        drop(temp_dir);
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    group.finish();
}

// =============================================================================
// End-to-End Latency (including IOWorker processing)
// =============================================================================

/// Benchmark end-to-end latency: append + wait for IOWorker to write + verify
/// readable This measures the TRUE latency including disk I/O
fn bench_e2e_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e_latency");
    group.sample_size(50);

    for &size in &[64, 256, 1024] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let queue = create_queue(&temp_dir, FlushMode::Sync);
                    let appender = queue.create_appender();
                    let msg = generate_message(size);
                    (temp_dir, queue, appender, msg)
                },
                |(temp_dir, queue, appender, msg)| {
                    // Append message
                    let seq = appender.append(black_box(msg)).unwrap();

                    // Wait until readable (IOWorker has processed it)
                    let mut tailer = queue.create_tailer_at(seq).unwrap();
                    loop {
                        if let Ok(Some(m)) = tailer.read_next()
                            && m.sequence == seq
                        {
                            black_box(m);
                            break;
                        }
                        std::thread::yield_now();
                    }

                    queue.shutdown().unwrap();
                    drop(temp_dir);
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

/// Benchmark raw mmap write + fsync latency (baseline, no queue overhead)
fn bench_raw_mmap_fsync(c: &mut Criterion) {
    use std::{fs::OpenOptions, io::Write};

    let mut group = c.benchmark_group("raw_mmap_fsync");
    group.sample_size(50);

    for &size in &[64, 256, 1024] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("write_fsync", size), &size, |b, &size| {
            b.iter_batched(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let path = temp_dir.path().join("test.data");
                    let file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .unwrap();
                    let data = vec![0xABu8; size];
                    (temp_dir, file, data)
                },
                |(_temp_dir, mut file, data)| {
                    file.write_all(black_box(&data)).unwrap();
                    file.sync_all().unwrap();
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

/// Benchmark IOWorker processing rate by measuring shutdown time
/// This shows how fast IOWorker can drain its queue
fn bench_ioworker_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("ioworker_drain");
    group.sample_size(20);

    let message_counts = [1000, 5000, 10000];
    let msg_size = 256;

    for &count in &message_counts {
        let total_bytes = (msg_size * count) as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let temp_dir = TempDir::new().unwrap();
                    let queue = create_queue(&temp_dir, FlushMode::Async);
                    let appender = queue.create_appender();
                    let msg = generate_message(msg_size);

                    // Pre-fill the channel
                    for _ in 0..count {
                        appender.append(msg.clone()).unwrap();
                    }

                    (temp_dir, queue)
                },
                |(temp_dir, queue)| {
                    // Measure shutdown time (IOWorker drains queue)
                    queue.shutdown().unwrap();
                    drop(temp_dir);
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

// =============================================================================
// Read (Tailer) Performance
// =============================================================================

/// Benchmark tailer read throughput
fn bench_tailer_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("tailer_read");
    group.sample_size(20);

    for &size in &[256, 1024, 4096] {
        let total_bytes = (size * BATCH_SIZE) as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter_batched(
                || {
                    // Setup: create queue and write messages
                    let temp_dir = TempDir::new().unwrap();
                    let queue = create_queue(&temp_dir, FlushMode::Async);
                    let appender = queue.create_appender();
                    let msg = generate_message(size);

                    for _ in 0..BATCH_SIZE {
                        appender.append(msg.clone()).unwrap();
                    }

                    // Small delay to let IOWorker process
                    std::thread::sleep(Duration::from_millis(100));

                    (temp_dir, queue)
                },
                |(temp_dir, queue)| {
                    // Benchmark: read all messages
                    let mut tailer = queue.create_tailer().unwrap();
                    let mut count = 0;
                    while let Ok(Some(msg)) = tailer.read_next() {
                        black_box(msg);
                        count += 1;
                        if count >= BATCH_SIZE {
                            break;
                        }
                    }
                    queue.shutdown().unwrap();
                    drop(temp_dir);
                },
                BatchSize::PerIteration,
            );
        });
    }

    group.finish();
}

// =============================================================================
// Criterion Configuration
// =============================================================================

criterion_group!(
    benches,
    bench_append_latency_async,
    bench_append_latency_sync,
    bench_append_latency_batch,
    bench_throughput_async,
    bench_throughput_batch,
    bench_append_batch_api,
    bench_e2e_latency,
    bench_raw_mmap_fsync,
    bench_ioworker_drain,
    bench_tailer_read,
);

criterion_main!(benches);
