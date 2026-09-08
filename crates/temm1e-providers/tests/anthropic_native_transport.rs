use serde_json::{json, Value};
use temm1e_core::{types::message::*, Provider};
use temm1e_providers::AnthropicProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn native_tool_continuation_survives_actual_http_and_serialized_history() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let native = json!([
        {"type":"thinking","thinking":"","signature":"fixture-signature"},
        {"type":"redacted_thinking","data":"fixture-redacted"},
        {"type":"tool_use","id":"call-1","name":"read_file","input":{"path":"a.txt"}}
    ]);
    let expected = native.clone();
    let server = tokio::spawn(async move {
        for turn in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
                assert!(head.len() < 65536);
            }
            let headers = String::from_utf8_lossy(&head).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(length < 65536);
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["thinking"]["type"], "adaptive");
            assert!(body.get("temperature").is_none());
            if turn == 1 {
                assert_eq!(body["messages"][1]["content"], expected);
                assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call-1");
            }
            let content = if turn == 0 {
                expected.clone()
            } else {
                json!([{"type":"text","text":"done"}])
            };
            let reply = json!({"id":format!("r-{turn}"),"type":"message","role":"assistant","model":"claude-fable-5-1","content":content,"stop_reason":if turn == 0 {"tool_use"} else {"end_turn"},"usage":{"input_tokens":20,"output_tokens":3}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
        }
    });
    let provider =
        AnthropicProvider::new("fixture".into()).with_base_url(format!("http://{address}"));
    let mut request = CompletionRequest {
        model: "claude-fable-5-1".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text("read a.txt".into()),
        }],
        tools: vec![],
        max_tokens: Some(128),
        temperature: Some(0.7),
        system: Some("fixture".into()),
        system_volatile: None,
    };
    let response = provider.complete(request.clone()).await.unwrap();
    let saved = serde_json::to_vec(&response.content).unwrap();
    let restored: Vec<ContentPart> = serde_json::from_slice(&saved).unwrap();
    assert!(restored.iter().any(|part| matches!(part, ContentPart::ProviderState { output, context_fingerprint: Some(_), .. } if output == native.as_array().unwrap())));
    request.messages.push(ChatMessage {
        role: Role::Assistant,
        content: MessageContent::Parts(restored),
    });
    request.messages.push(ChatMessage {
        role: Role::User,
        content: MessageContent::Parts(vec![ContentPart::ToolResult {
            tool_use_id: "call-1".into(),
            content: "file content".into(),
            is_error: false,
        }]),
    });
    let response = provider.complete(request).await.unwrap();
    assert!(response
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::Text { text } if text == "done")));
    server.await.unwrap();
}
