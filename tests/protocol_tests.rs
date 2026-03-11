use tako_ipc::protocol::{
    ErrorBody, MessageType, PROTOCOL_VERSION, RequestEnvelope, ResponseEnvelope,
};

#[test]
fn protocol_version_is_frozen() {
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn failed_response_requires_error() {
    let response = ResponseEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Response,
        request_id: "req-1".into(),
        ok: false,
        payload: None,
        error: Some(ErrorBody {
            code: "internal_error".into(),
            message: "boom".into(),
            details: None,
        }),
        trace_id: None,
        metadata: None,
    };

    assert!(response.validate().is_ok());
}

#[test]
fn successful_response_must_not_have_error() {
    let response = ResponseEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Response,
        request_id: "req-1".into(),
        ok: true,
        payload: Some(vec![1, 2, 3]),
        error: Some(ErrorBody {
            code: "invalid_request".into(),
            message: "unexpected".into(),
            details: None,
        }),
        trace_id: None,
        metadata: None,
    };

    assert!(response.validate().is_err());
}

#[test]
fn request_envelope_requires_expected_shape() {
    let request = RequestEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Request,
        request_id: "req-1".into(),
        method: "ping".into(),
        deadline_ms: None,
        trace_id: None,
        payload: vec![1],
        metadata: None,
    };

    assert!(request.validate().is_ok());
}

#[test]
fn successful_response_requires_payload() {
    let response = ResponseEnvelope {
        version: PROTOCOL_VERSION,
        message_type: MessageType::Response,
        request_id: "req-1".into(),
        ok: true,
        payload: None,
        error: None,
        trace_id: None,
        metadata: None,
    };

    assert!(response.validate().is_err());
}
