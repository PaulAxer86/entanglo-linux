use entanglo_core::protocol::message_type::MessageType;
use entanglo_core::protocol::payloads::HelloPayload;
use entanglo_core::protocol::EntangloMessage;

#[test]
fn hello_roundtrips() {
    let payload = HelloPayload {
        device_name: "LinuxDev".into(),
        device_model: "Linux".into(),
        app_version: "0.1.0".into(),
        roles: vec!["controller".into(), "receiver".into()],
        platform: Some("Linux".into()),
        screen_width: Some(1920),
        screen_height: Some(1080),
    };
    let msg = EntangloMessage::encode_payload(MessageType::Hello, "dev-id", "sess-id", &payload)
        .expect("encode");
    let bytes = serde_json::to_vec(&msg).expect("serialize envelope");
    let back = EntangloMessage::decode(&bytes).expect("decode envelope");
    let decoded: HelloPayload = back.decode_payload().expect("decode payload");
    assert_eq!(payload, decoded);
    assert_eq!(decoded.screen_size(), Some((1920.0, 1080.0)));
}

#[test]
fn hello_without_screen_size_remains_backward_compatible() {
    let json = br#"{
        "deviceName":"OldPeer",
        "deviceModel":"Mac",
        "appVersion":"0.1.58",
        "roles":["receiver"],
        "platform":"macOS"
    }"#;
    let decoded: HelloPayload = serde_json::from_slice(json).expect("legacy hello");
    assert_eq!(decoded.screen_size(), None);
}

#[test]
fn rejects_unsupported_protocol_version() {
    let json = br#"{
        "protocolVersion": 2,
        "messageType": "hello",
        "senderDeviceId": "x",
        "sessionId": "y",
        "timestamp": 0.0,
        "payload": ""
    }"#;
    let result = EntangloMessage::decode(json);
    assert!(
        result.is_err(),
        "protocolVersion=2 must be rejected per PROTOCOL.md §8"
    );
}

#[test]
fn heartbeat_missing_clickstate_style_optional_fields_omit_from_wire() {
    // Mirrors the "receiver MUST treat missing clickState as 1"
    // forward-compat rule (§5.5) by checking optional fields really
    // are omitted rather than serialized as `null`, matching Swift
    // Codable's default behaviour that peers rely on.
    use entanglo_core::protocol::payloads::input_event::{InputEventKind, InputEventMessage};
    let event = InputEventMessage {
        kind: InputEventKind::MouseMove,
        x: Some(1.0),
        y: Some(2.0),
        delta_x: None,
        delta_y: None,
        button: None,
        key_code: None,
        media_key: None,
        modifier_flags: 0,
        pressed: None,
        click_state: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains("clickState"));
    assert!(!json.contains("keyCode"));
}
