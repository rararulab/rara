# Queue Manifest Design

## Overview

Introduce a manifest mechanism to the persistent queue for O(1) recovery instead of O(n) full file scanning.

**Problem**: Current recovery scans all data files sequentially. With tens of thousands of files, startup time becomes unacceptable.

**Solution**: Maintain a manifest file recording queue metadata. Recovery reads manifest + incrementally scans only the active file.

## File Layout

```
base_path/
├── manifest.1          # Dual-file alternating write
├── manifest.2
├── manifest.current    # 1 byte: 0x01 or 0x02, indicates valid slot
└── YYYY/MM/DD/*.data   # Data files
```

## Manifest Binary Format

Little-endian encoding throughout.

```
┌────────────────────────────────────────────────────────┐
│ Header (32 bytes)                                       │
├─────────────────┬──────────────────────────────────────┤
│ magic: [u8; 4]  │ "QMFT" (0x514D4654)                  │
│ version: u32    │ Format version, currently 1          │
│ next_seq: u64   │ Next global sequence number          │
│ file_count: u32 │ Number of FileEntry records          │
│ checksum: u32   │ CRC32 of entire file content         │
│ reserved: [u8;8]│ Reserved for future use              │
├─────────────────┴──────────────────────────────────────┤
│ Active File State (variable)                           │
├─────────────────┬──────────────────────────────────────┤
│ file_seq: u32   │ Current active file sequence         │
│ write_pos: u64  │ Write offset in active file          │
│ msg_count: u64  │ Message count in active file         │
│ path_len: u16   │ Path string length                   │
│ path: [u8; N]   │ UTF-8 path string                    │
├─────────────────┴──────────────────────────────────────┤
│ FileEntry[] (variable)                                 │
│ Per entry:                                             │
│   start_seq: u64   - First message sequence            │
│   end_seq: u64     - Last message sequence             │
│   size: u64        - File size in bytes                │
│   path_len: u16    - Path string length                │
│   path: [u8; N]    - UTF-8 path string                 │
│ Total: ~26 + path_len bytes per entry                  │
└────────────────────────────────────────────────────────┘
```

**Capacity estimate**: 10,000 files with ~50 byte paths → ~760KB. Acceptable for full rewrite.

## Dual-File Write Strategy

### Why Dual-File?

- **Atomic**: Always have at least one valid manifest
- **Simple**: No WAL replay logic, no compaction
- **Suitable for low-frequency updates**: Roll file is infrequent (seconds or longer intervals)

### Write Procedure

```
1. Read manifest.current → assume current is 1
2. Build new manifest content in memory
3. Calculate checksum
4. Write to manifest.2 (backup slot)
5. fsync manifest.2
6. Write manifest.current = 0x02
7. fsync manifest.current
```

### Crash Recovery Analysis

| Crash Point | manifest.current | manifest.1 | manifest.2 | Recovery |
|-------------|------------------|------------|------------|----------|
| During step 4 | 0x01 | Valid ✓ | Corrupted | Use manifest.1 |
| After 5, before 6 | 0x01 | Valid ✓ | Valid | Use manifest.1 |
| After 6, before 7 | Unknown | Valid ✓ | Valid ✓ | Either works |
| After step 7 | 0x02 | Old | Valid ✓ | Use manifest.2 |

**Key invariant**: At any crash point, at least one complete valid manifest exists.

## Update Timing

Manifest is updated:
- After each file roll (new file created)
- On graceful shutdown (record final state)

**Not updated on every message write** — this is intentional. write_position and message_count for the active file can be recovered by scanning only that file, which is O(1) not O(n).

## Recovery Flow

```
1. manifest.current does not exist
   → Fresh queue, initialize with defaults

2. Read manifest.current → get slot number (1 or 2)

3. Read and validate manifest.{slot}
   - Checksum passes → use this manifest
   - Checksum fails → Error::ManifestCorrupted (no fallback)

4. Restore state from manifest
   - next_sequence, file_sequence: use directly
   - write_position, message_count: verify via active file scan

5. Incremental scan of active file
   - Start from manifest.write_position
   - Count additional valid messages
   - Update next_sequence accordingly
```

**Time complexity**: O(1) manifest read + O(k) active file scan, where k is messages written since last manifest update (typically small, at most one file's worth).

## Code Structure

### New Files

```
crates/common/storage/queue/src/
├── manifest.rs        # Manifest struct, serialization/deserialization
├── manifest_writer.rs # Dual-file write logic, atomicity guarantees
└── (modify) recovery.rs  # Integrate manifest recovery path
```

### Core Types

```rust
// manifest.rs

pub struct Manifest {
    pub version: u32,
    pub next_sequence: u64,
    pub active_file: ActiveFileState,
    pub files: Vec<FileEntry>,
}

pub struct ActiveFileState {
    pub file_sequence: u32,
    pub write_position: u64,
    pub message_count: u64,
    pub path: PathBuf,
}

pub struct FileEntry {
    pub path: PathBuf,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub size: u64,
}
```

```rust
// manifest_writer.rs

pub struct ManifestWriter {
    base_path: PathBuf,
    current_slot: u8,  // 1 or 2
}

impl ManifestWriter {
    pub fn write(&mut self, manifest: &Manifest) -> Result<()>;
    pub fn read_latest(&self) -> Result<Option<Manifest>>;
}
```

### Integration Points

| Component | Modification |
|-----------|--------------|
| `Queue::new()` | Recover from manifest |
| `IOWorker::roll_file()` | Call `ManifestWriter::write()` after roll |
| `Queue::shutdown()` | Write final manifest state |

## Error Handling

### New Error Variants

```rust
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Manifest corrupted: {reason}"))]
    ManifestCorrupted { reason: String },

    #[snafu(display("Manifest version {version} not supported"))]
    UnsupportedManifestVersion { version: u32 },

    #[snafu(display("Manifest not found, expected at {path}"))]
    ManifestNotFound { path: PathBuf },
}
```

### Error Scenarios

| Scenario | Handling |
|----------|----------|
| First startup (no manifest) | Create empty manifest, file_count = 0 |
| manifest.current exists but points to corrupted file | Error::ManifestCorrupted |
| Checksum validation fails | Error::ManifestCorrupted |
| Unsupported version | Error::UnsupportedManifestVersion |

### Design Principles

- **No fallback to full scan**: Manifest corruption is a hard error
- **Manifest is source of truth**: No consistency checks against actual files
- **External interference is user's responsibility**: Manual file deletion/modification not handled

## Summary

| Aspect | Design Choice |
|--------|---------------|
| Storage strategy | Dual-file alternating write |
| Update frequency | On roll file + shutdown |
| Recovery complexity | O(1) + O(k) |
| Fallback on corruption | None (hard error) |
| File consistency check | None (trust manifest) |
