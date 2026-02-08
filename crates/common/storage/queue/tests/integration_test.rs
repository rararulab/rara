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

use std::time::Duration;

use bytes::Bytes;
use rsketch_common_storage_queue::{FlushMode, QueueBuilder, RollStrategy};
use tempfile::TempDir;

#[test]
fn test_queue_write_and_read() {
    let temp_dir = TempDir::new().unwrap();

    let queue = QueueBuilder::new(temp_dir.path())
        .file_size(1024 * 1024)
        .flush_mode(FlushMode::Sync)
        .build()
        .unwrap();

    let appender = queue.create_appender();

    for i in 0..100 {
        let msg = format!("message-{i:04}");
        let seq = appender.append(msg).unwrap();
        assert_eq!(seq, i);
    }

    std::thread::sleep(Duration::from_millis(50));

    let mut tailer = queue.create_tailer().unwrap();

    for i in 0..100 {
        let msg = tailer.read_next().unwrap().unwrap();
        assert_eq!(msg.sequence, i);
        assert_eq!(
            std::str::from_utf8(&msg.payload).unwrap(),
            format!("message-{i:04}")
        );
    }

    assert!(tailer.read_next().unwrap().is_none());

    queue.shutdown().unwrap();
}

#[test]
fn test_queue_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().to_path_buf();

    {
        let queue = QueueBuilder::new(&path)
            .file_size(1024 * 1024)
            .flush_mode(FlushMode::Sync)
            .build()
            .unwrap();

        let appender = queue.create_appender();
        for i in 0..50 {
            appender.append(format!("msg-{i}")).unwrap();
        }

        std::thread::sleep(Duration::from_millis(50));
        queue.shutdown().unwrap();
    }

    {
        let queue = QueueBuilder::new(&path)
            .file_size(1024 * 1024)
            .flush_mode(FlushMode::Sync)
            .build()
            .unwrap();

        assert_eq!(queue.current_sequence(), 50);

        let appender = queue.create_appender();
        for i in 50..100 {
            let seq = appender.append(format!("msg-{i}")).unwrap();
            assert_eq!(seq, i);
        }

        std::thread::sleep(Duration::from_millis(50));

        let mut tailer = queue.create_tailer().unwrap();
        for i in 0..100 {
            let msg = tailer.read_next().unwrap().unwrap();
            assert_eq!(msg.sequence, i);
        }

        queue.shutdown().unwrap();
    }
}

#[test]
fn test_queue_batch_append() {
    let temp_dir = TempDir::new().unwrap();

    let queue = QueueBuilder::new(temp_dir.path())
        .file_size(1024 * 1024)
        .flush_mode(FlushMode::Sync)
        .build()
        .unwrap();

    let appender = queue.create_appender();

    let messages: Vec<Bytes> = (0..10)
        .map(|i| Bytes::from(format!("batch-msg-{i}")))
        .collect();

    let sequences = appender.append_batch(messages).unwrap();
    assert_eq!(sequences, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

    std::thread::sleep(Duration::from_millis(50));

    let mut tailer = queue.create_tailer().unwrap();
    for i in 0..10 {
        let msg = tailer.read_next().unwrap().unwrap();
        assert_eq!(msg.sequence, i);
        assert_eq!(
            std::str::from_utf8(&msg.payload).unwrap(),
            format!("batch-msg-{i}")
        );
    }

    queue.shutdown().unwrap();
}

#[test]
fn test_queue_tailer_seek() {
    let temp_dir = TempDir::new().unwrap();

    let queue = QueueBuilder::new(temp_dir.path())
        .file_size(1024 * 1024)
        .flush_mode(FlushMode::Sync)
        .index_interval(10)
        .build()
        .unwrap();

    let appender = queue.create_appender();

    for i in 0..100 {
        appender.append(format!("seek-msg-{i:04}")).unwrap();
    }

    std::thread::sleep(Duration::from_millis(50));

    let mut tailer = queue.create_tailer_at(50).unwrap();

    let msg = tailer.read_next().unwrap().unwrap();
    assert_eq!(msg.sequence, 50);
    assert_eq!(std::str::from_utf8(&msg.payload).unwrap(), "seek-msg-0050");

    queue.shutdown().unwrap();
}

#[test]
fn test_queue_file_rolling() {
    let temp_dir = TempDir::new().unwrap();

    let queue = QueueBuilder::new(temp_dir.path())
        .file_size(1024)
        .roll_strategy(RollStrategy::ByCount(10))
        .flush_mode(FlushMode::Sync)
        .build()
        .unwrap();

    let appender = queue.create_appender();

    for i in 0..25 {
        appender.append(format!("roll-msg-{i:04}")).unwrap();
    }

    std::thread::sleep(Duration::from_millis(100));

    let mut tailer = queue.create_tailer().unwrap();

    for i in 0..25 {
        let msg = tailer.read_next().unwrap().unwrap();
        assert_eq!(msg.sequence, i);
    }

    queue.shutdown().unwrap();
}

#[test]
fn test_queue_iterator() {
    let temp_dir = TempDir::new().unwrap();

    let queue = QueueBuilder::new(temp_dir.path())
        .file_size(1024 * 1024)
        .flush_mode(FlushMode::Sync)
        .build()
        .unwrap();

    let appender = queue.create_appender();
    for i in 0..10 {
        appender.append(format!("iter-msg-{i}")).unwrap();
    }

    std::thread::sleep(Duration::from_millis(50));

    let tailer = queue.create_tailer().unwrap();
    let messages: Vec<_> = tailer.take(10).collect();

    assert_eq!(messages.len(), 10);
    for (i, result) in messages.into_iter().enumerate() {
        let msg = result.unwrap();
        assert_eq!(msg.sequence, i as u64);
    }

    queue.shutdown().unwrap();
}
