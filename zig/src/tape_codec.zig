// Tape JSONL codec — Zig 0.16 implementation.
//
// FFI surface (matches lib.rs):
//
//   tape_codec_zig_decode(in_ptr, in_len, out_ptr, out_capacity, out_written) -> i32
//   tape_codec_zig_encode(in_ptr, in_len, out_ptr, out_capacity, out_written) -> i32
//
// Both functions are caller-allocated: the Rust side passes a buffer and
// receives the number of bytes written. No Zig allocator state crosses the
// boundary. A fresh ArenaAllocator is constructed inside each call and freed
// before the function returns.
//
// Why JSON-bytes-in / JSON-bytes-out instead of mirroring `TapEntry` as a
// Zig struct: the entry's `payload` is `serde_json::Value` (arbitrary JSON),
// which is impractical to mirror as a Zig type. The differential test
// (`tests/differential.rs`) calls `decode` on serde-emitted bytes, then
// `encode` on the resulting parsed `Value`, and asserts byte-equality.
//
// Byte-parity strategy:
//
//   1. `std.json.ObjectMap` is a `StringArrayHashMap` which preserves
//      insertion order — i.e. round-tripping a JSON object preserves
//      the original key order in the input bytes. That is exactly what
//      we need to match serde:
//
//        - For struct shapes (`TapEntry`), serde's `Serialize` derive
//          emits fields in declaration order; Zig sees that order and
//          re-emits it.
//        - For `Value::Object` payloads (`BTreeMap`), serde emits keys
//          in lexicographic order; Zig sees that already-sorted order
//          and re-emits it.
//
//      In other words, this codec does NOT need to sort anything — it
//      just needs to NOT reorder. The first prototype eagerly sorted
//      keys, which broke struct-shape parity; the differential test
//      caught it.
//
//   2. `parse_numbers = false` keeps numeric values as their original
//      byte slice (`Value.number_string`). Without this, Zig parses
//      `722.0` into an `f64`, and stringifies it as `722` — diverging
//      from serde, which preserves the `.0`. Likewise for scientific
//      notation, leading zeros after the decimal, etc.
//
//   3. `escape_unicode = false` (the default) emits non-ASCII bytes
//      verbatim, matching serde's default. If we ever flip serde to
//      ASCII-escape, this option must flip too.

const std = @import("std");

const RC_OK: i32 = 0;
const RC_INVALID_JSON: i32 = 1;
const RC_OUT_OF_MEMORY: i32 = 2;
const RC_BUFFER_TOO_SMALL: i32 = 3;

/// Round-trip JSON bytes through `std.json.Value`. See file-level
/// comment for the byte-parity strategy.
fn roundTrip(in_bytes: []const u8, out_buf: []u8, out_written: *usize) i32 {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const a = arena.allocator();

    const parsed = std.json.parseFromSliceLeaky(
        std.json.Value,
        a,
        in_bytes,
        .{ .parse_numbers = false },
    ) catch |err| {
        return switch (err) {
            error.OutOfMemory => RC_OUT_OF_MEMORY,
            else => RC_INVALID_JSON,
        };
    };

    const out = std.json.Stringify.valueAlloc(a, parsed, .{}) catch return RC_OUT_OF_MEMORY;

    if (out.len > out_buf.len) {
        out_written.* = out.len;
        return RC_BUFFER_TOO_SMALL;
    }
    @memcpy(out_buf[0..out.len], out);
    out_written.* = out.len;
    return RC_OK;
}

export fn tape_codec_zig_decode(
    in_ptr: [*]const u8,
    in_len: usize,
    out_ptr: [*]u8,
    out_capacity: usize,
    out_written: *usize,
) i32 {
    const in_bytes = in_ptr[0..in_len];
    const out_buf = out_ptr[0..out_capacity];
    return roundTrip(in_bytes, out_buf, out_written);
}

export fn tape_codec_zig_encode(
    in_ptr: [*]const u8,
    in_len: usize,
    out_ptr: [*]u8,
    out_capacity: usize,
    out_written: *usize,
) i32 {
    const in_bytes = in_ptr[0..in_len];
    const out_buf = out_ptr[0..out_capacity];
    return roundTrip(in_bytes, out_buf, out_written);
}
