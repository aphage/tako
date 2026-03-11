use tako_ipc::codec::{decode_frame, encode_frame, validate_length, CodecError};
use tako_ipc::codec::{decode_cbor, encode_cbor};
use tako_ipc::protocol::MAX_FRAME_SIZE;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct DemoValue {
    value: String,
}

#[test]
fn frame_roundtrip_preserves_payload() {
    let payload = br#"hello"#;
    let frame = encode_frame(payload).expect("frame should encode");
    let decoded = decode_frame(&frame).expect("frame should decode");
    assert_eq!(decoded, payload);
}

#[test]
fn zero_length_is_invalid() {
    let err = validate_length(0).expect_err("zero length must fail");
    assert_eq!(err, CodecError::InvalidLength(0));
}

#[test]
fn oversized_frame_is_rejected() {
    let err = validate_length(MAX_FRAME_SIZE + 1).expect_err("oversized frame must fail");
    assert_eq!(err, CodecError::FrameTooLarge(MAX_FRAME_SIZE + 1));
}

#[test]
fn cbor_roundtrip_preserves_structured_payload() {
    let value = DemoValue {
        value: "hello".into(),
    };
    let bytes = encode_cbor(&value).expect("cbor should encode");
    let decoded: DemoValue = decode_cbor(&bytes).expect("cbor should decode");
    assert_eq!(decoded, value);
}

#[test]
fn invalid_cbor_is_rejected() {
    let err = decode_cbor::<DemoValue>(&[0xff]).expect_err("invalid cbor must fail");
    assert_eq!(err, CodecError::InvalidCbor);
}
