use std::io::{BufReader, Cursor, Read};

use harness_core::framing::{
    FrameError, MAX_FRAME_BYTES, decode_frame, encode_frame, read_frame, write_frame,
};
use harness_core::protocol::Envelope;

const HELLO: &str = include_str!("../../../tests/fixtures/protocol/valid/hello.json");
const COMPLETED: &str = include_str!("../../../tests/fixtures/protocol/valid/completed.json");

fn lines(parts: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in parts {
        v.extend_from_slice(p.trim().as_bytes());
        v.push(b'\n');
    }
    v
}

#[test]
fn roundtrip_multiple_frames() {
    let a = Envelope::from_json(HELLO).unwrap();
    let b = Envelope::from_json(COMPLETED).unwrap();
    let mut buf = Vec::new();
    write_frame(&mut buf, &a).unwrap();
    write_frame(&mut buf, &b).unwrap();
    assert_eq!(buf.iter().filter(|c| **c == b'\n').count(), 2);

    let mut r = Cursor::new(buf);
    assert_eq!(read_frame(&mut r).unwrap().unwrap(), a);
    assert_eq!(read_frame(&mut r).unwrap().unwrap(), b);
    assert!(read_frame(&mut r).unwrap().is_none());
}

#[test]
fn rejects_bad_json() {
    for bad in ["{not json", "[]", "null", "{\"kind\":\"hello\"}", "   "] {
        let mut data = lines(&[bad]);
        data.push(b'\n');
        let mut r = Cursor::new(data);
        let err = read_frame(&mut r).unwrap_err();
        assert!(matches!(err, FrameError::BadFrame(_)), "{bad}: {err}");
    }
    // Invalid UTF-8.
    let mut r = Cursor::new(vec![0xff, 0xfe, b'\n']);
    assert!(matches!(
        read_frame(&mut r).unwrap_err(),
        FrameError::BadFrame(_)
    ));
    // Contract violations are bad frames too.
    let invalid = include_str!("../../../tests/fixtures/protocol/invalid/extra_field.json");
    let mut r = Cursor::new(lines(&[invalid]));
    assert!(matches!(
        read_frame(&mut r).unwrap_err(),
        FrameError::BadFrame(_)
    ));
}

#[test]
fn rejects_oversize_frame() {
    let big = vec![b'a'; MAX_FRAME_BYTES + 1];
    let mut data = big.clone();
    data.push(b'\n');
    assert!(matches!(
        read_frame(&mut Cursor::new(data)).unwrap_err(),
        FrameError::Oversize { .. }
    ));
    // Detected without a newline, and with a tiny buffer so the limit is enforced
    // incrementally rather than after buffering everything.
    let mut r = BufReader::with_capacity(16, Cursor::new(big));
    assert!(matches!(
        read_frame(&mut r).unwrap_err(),
        FrameError::Oversize { .. }
    ));
    assert!(matches!(
        decode_frame(&vec![b'x'; MAX_FRAME_BYTES + 1]).unwrap_err(),
        FrameError::Oversize { .. }
    ));
}

#[test]
fn frame_at_exact_limit_is_accepted_by_size_check() {
    // A body of exactly the limit passes the size check (and then fails JSON parsing).
    let mut data = vec![b' '; MAX_FRAME_BYTES];
    data.push(b'\n');
    assert!(matches!(
        read_frame(&mut Cursor::new(data)).unwrap_err(),
        FrameError::BadFrame(_)
    ));
}

#[test]
fn oversize_envelope_cannot_be_written() {
    let mut e = Envelope::from_json(HELLO).unwrap();
    e.payload.insert(
        "blob".into(),
        serde_json::Value::String("a".repeat(MAX_FRAME_BYTES)),
    );
    assert!(matches!(
        encode_frame(&e).unwrap_err(),
        FrameError::Oversize { .. }
    ));
}

#[test]
fn truncated_stream_and_crlf() {
    let mut half = HELLO.trim().as_bytes().to_vec();
    assert!(matches!(
        read_frame(&mut Cursor::new(half.clone())).unwrap_err(),
        FrameError::Truncated
    ));
    half.extend_from_slice(b"\r\n");
    assert!(read_frame(&mut Cursor::new(half)).unwrap().is_some());
}

#[test]
fn reads_small_chunks() {
    struct OneByte<R: Read>(R);
    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }
    let data = lines(&[HELLO, COMPLETED]);
    let mut r = BufReader::with_capacity(1, OneByte(Cursor::new(data)));
    assert!(read_frame(&mut r).unwrap().is_some());
    assert!(read_frame(&mut r).unwrap().is_some());
    assert!(read_frame(&mut r).unwrap().is_none());
}
