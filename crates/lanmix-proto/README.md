# lanmix-proto

Dependency-free Rust 2024 codec for LanMix protocol v1. Encoding writes into a
caller-provided buffer; decoding borrows the datagram and exposes an exact-size
iterator of `[f32; 2]` stereo frames. Neither operation allocates. No unsafe code,
native-layout casts, ordering logic, or network buffering is used.

## Wire format

All multibyte fields use explicit little-endian byte order. The header is exactly
40 bytes. Byte ranges below are half-open.

| Bytes | Field | Type / required value |
| --- | --- | --- |
| 0..2 | magic | ASCII `LM` |
| 2 | version | u8, `1` |
| 3 | flags | u8, `0` |
| 4..12 | session_id | u64 |
| 12..16 | stream_id | u32 |
| 16..20 | seq | u32 |
| 20..28 | frame_index | u64 |
| 28..32 | sample_rate | u32, 8000 through 192000 inclusive |
| 32..34 | frame_count | u16, 0 through 145 inclusive |
| 34 | channels | u8, `2` |
| 35 | format | u8, `1` (IEEE 754 f32, little-endian) |
| 36..40 | reserved | u32, `0` |

The payload contains exactly `frame_count` interleaved left/right frames, each
8 bytes: four bytes for the left sample followed by four for the right sample.
Datagram size must equal `40 + frame_count * 8` and must not exceed 1200 bytes.
The maximum payload is 145 frames (1160 bytes). Zero-frame packets are valid.
The exclusive end index `frame_index + frame_count` must fit in `u64`; both
encoding and decoding reject overflow. An end index equal to `u64::MAX` is valid,
including a zero-frame packet starting at `u64::MAX`.
All samples must be finite; NaNs and positive/negative infinity are rejected.
Finite values outside [-1, 1], subnormals, and signed zeros retain their bits.

Decoding rejects every unsupported fixed field, malformed length, excessive
frame count, invalid sample rate, and nonfinite sample before returning a packet.
Encoding requires the header count to equal the supplied frame count, applies the
same rate/count/sample rules, and emits all fixed fields with their required
values. Encoder errors leave the output buffer unchanged. Extra output capacity
is allowed and remains untouched after the returned byte count.

`Packet::frames()` can be called repeatedly, uses the validated payload bounds,
and stays within those bounds if the caller later modifies the public header.
Session management, sequence interpretation, packet reordering, and loss handling
belong to the parent application.

The current sender increments `seq` once for each original datagram attempt,
including failed socket sends; it wraps modulo 2^32. A delayed backup repeats
the entire datagram byte for byte, including session, sequence and frame index.
It does not consume a new sequence number. Within a session, consecutive
sequences with a forward frame-index gap identify a sender timeline omission;
missing sequences can result from send failures, network loss or expiry at the
receiver. Classification uses playback order after the reorder deadline, never
raw arrival order. Older senders without backups remain compatible. The wire
format has not changed.

## Example

```rust
use lanmix_proto::{Header, MAX_DATAGRAM_BYTES, decode, encode};

let header = Header {
    session_id: 1,
    stream_id: 0,
    seq: 7,
    frame_index: 1024,
    sample_rate: 48_000,
    frame_count: 2,
};
let frames = [[0.25, -0.25], [0.5, -0.5]];
let mut buffer = [0; MAX_DATAGRAM_BYTES];
let len = encode(&header, &frames, &mut buffer)?;
let packet = decode(&buffer[..len])?;
assert_eq!(packet.header, header);
assert!(packet.frames().eq(frames));
# Ok::<(), lanmix_proto::ProtocolError>(())
```
