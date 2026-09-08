use std::{sync::Arc, time::Duration};
use temm1e_core::{types::message::*, Provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn request() -> CompletionRequest {
    CompletionRequest {
        model: "fixture".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: Some(32),
        temperature: None,
        system: None,
        system_volatile: None,
    }
}

#[tokio::test]
async fn real_transport_delivers_unicode_before_completion_and_retains_late_usage() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let seen_tx = Arc::new(std::sync::Mutex::new(Some(seen_tx)));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request_bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            request_bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = request_bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request_bytes[..end]).to_ascii_lowercase();
                let len: usize = headers
                    .lines()
                    .find_map(|s| s.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request_bytes.len() >= end + 4 + len {
                    let body: serde_json::Value =
                        serde_json::from_slice(&request_bytes[end + 4..end + 4 + len]).unwrap();
                    assert_eq!(body["stream"], true);
                    assert!(
                        body.get("stream_options").is_none(),
                        "custom endpoint must not receive undocumented options"
                    );
                    break;
                }
            }
        }
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let first = "data: {\"id\":\"r1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"café 🐈\"},\"finish_reason\":null}]}\r\n\r\n";
        for byte in first.as_bytes() {
            socket.write_all(&[*byte]).await.unwrap();
        }
        // The server will not finish until the client's actual observer receives text.
        tokio::time::timeout(Duration::from_secs(3), seen_rx)
            .await
            .unwrap()
            .unwrap();
        socket.write_all(b"data: {\"id\":\"r1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"id\":\"r1\",\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":10,\"cache_write_tokens\":4}}}\n\ndata: [DONE]\n\n").await.unwrap();
    });
    let provider = temm1e_providers::OpenAICompatProvider::new("fixture".into())
        .with_base_url(format!("http://{address}"));
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        provider.complete_with_observer(
            request(),
            Arc::new(move |text| {
                assert_eq!(text, "café 🐈");
                if let Some(tx) = seen_tx.lock().unwrap().take() {
                    tx.send(()).unwrap();
                }
            }),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.id, "r1");
    assert_eq!(response.usage.input_tokens, 20);
    assert_eq!(response.usage.output_tokens, 3);
    assert_eq!(response.usage.cache_read_tokens, Some(10));
    assert_eq!(response.usage.cache_write_tokens, Some(4));
    assert_eq!(response.usage.totals_reported, Some(true));
    assert!(matches!(&response.content[..], [ContentPart::Text { text }] if text == "café 🐈"));
    server.await.unwrap();
}

#[tokio::test]
async fn truncated_or_malformed_wire_response_never_returns_a_completed_tool_call() {
    for body in [
        "data: {\"id\":\"r\",\"choices\":[{\"delta\":{\"content\":\"unfinished\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"r\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"x\",\"function\":{\"name\":\"shell\",\"arguments\":\"{bad\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
        "data: {\"id\":\"r\",\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\ndata: [DONE]\n\n",
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 65_536);
                if request.ends_with(b"\r\n\r\n") { break; }
            }
            let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let length: usize = headers.lines().find_map(|s| s.strip_prefix("content-length:")).unwrap().trim().parse().unwrap();
            socket.read_exact(&mut vec![0; length]).await.unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let provider = temm1e_providers::OpenAICompatProvider::new("fixture".into()).with_base_url(format!("http://{address}"));
        let result = tokio::time::timeout(Duration::from_secs(3), provider.complete_with_observer(request(), Arc::new(|_| {}))).await.unwrap();
        assert!(result.is_err(), "partial or invalid response must not authorize tool dispatch");
        server.await.unwrap();
    }
}

#[tokio::test]
async fn native_anthropic_transport_requires_message_stop_and_merges_usage() {
    for complete in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            loop {
                let mut b = [0];
                socket.read_exact(&mut b).await.unwrap();
                headers.push(b[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(headers.len() < 65_536);
            }
            let headers = String::from_utf8_lossy(&headers).to_ascii_lowercase();
            assert!(headers.starts_with("post /v1/messages "));
            let n: usize = headers
                .lines()
                .find_map(|s| s.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            socket.read_exact(&mut vec![0; n]).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
            let events = [
                serde_json::json!({"type":"message_start","message":{"id":"m1","usage":{"input_tokens":11,"output_tokens":1,"cache_read_input_tokens":4}}}),
                serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"café 🐈"}}),
                serde_json::json!({"type":"content_block_stop","index":0}),
                serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}),
            ];
            for value in events {
                let event = format!(
                    "event: {}\r\ndata: {}\r\n\r\n",
                    value["type"].as_str().unwrap(),
                    value
                );
                for byte in event.as_bytes() {
                    socket.write_all(&[*byte]).await.unwrap();
                }
            }
            if complete {
                socket
                    .write_all(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
                    .await
                    .unwrap();
            }
        });
        let provider = temm1e_providers::AnthropicProvider::new("fixture".into())
            .with_base_url(format!("http://{address}"));
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let target = seen.clone();
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            provider.complete_with_observer(
                request(),
                Arc::new(move |s| target.lock().unwrap().push_str(s)),
            ),
        )
        .await
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), "café 🐈");
        if complete {
            let response = response.unwrap();
            assert_eq!(response.id, "m1");
            assert_eq!(response.usage.input_tokens, 15);
            assert_eq!(response.usage.output_tokens, 7);
            assert_eq!(response.usage.cache_read_tokens, Some(4));
        } else {
            assert!(
                response.is_err(),
                "a stop reason alone does not confirm message_stop or final usage"
            );
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn openai_native_transport_round_trips_reasoning_phase_and_tools_across_requests() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let native = serde_json::json!([
        {"id":"rs","type":"reasoning","encrypted_content":"opaque-session-state","summary":[]},
        {"id":"msg","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Inspecting café","annotations":[]}]},
        {"id":"fc","type":"function_call","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"a.txt\"}"}
    ]);
    let expected_native = native.clone();
    let server = tokio::spawn(async move {
        for turn in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                assert!(headers.len() <= 16384);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
            assert!(headers.starts_with("post /v1/responses http/1.1\r\n"));
            let len: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(len < 65536);
            let mut raw = vec![0; len];
            socket.read_exact(&mut raw).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            assert_eq!(body["model"], "gpt-6-astra");
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert_eq!(body["max_output_tokens"], 32);
            assert!(body.get("temperature").is_none());
            assert!(body.get("messages").is_none());
            if turn == 1 {
                assert_eq!(
                    &body["input"].as_array().unwrap()[..3],
                    expected_native.as_array().unwrap()
                );
                assert_eq!(body["input"][3]["call_id"], "call_1");
                assert_eq!(body["input"][3]["type"], "function_call_output");
            }
            let output = if turn == 0 {
                expected_native.clone()
            } else {
                serde_json::json!([{"id":"final","type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Done","annotations":[]}]}])
            };
            let data = serde_json::json!({"type":"response.completed","response":{"id":format!("resp_{turn}"),"status":"completed","output":output,"usage":{"input_tokens":40,"output_tokens":10,"input_tokens_details":{"cached_tokens":20}}}});
            let wire = format!("event: response.completed\r\ndata: {data}\r\n\r\n");
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",wire.len()).as_bytes()).await.unwrap();
            for byte in wire.bytes() {
                socket.write_all(&[byte]).await.unwrap();
            }
        }
    });
    let provider = temm1e_providers::OpenAICompatProvider::new("fixture".into())
        .with_name("openai")
        .with_base_url(format!("http://{address}/v1/"));
    let mut req = request();
    req.model = "gpt-6-astra".into();
    req.temperature = Some(0.7);
    let first = tokio::time::timeout(Duration::from_secs(5), provider.complete(req.clone()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.usage.input_tokens, 40);
    assert_eq!(first.usage.cache_read_tokens, Some(20));
    assert!(
        matches!(&first.content[2], ContentPart::ProviderState { output, .. } if output == native.as_array().unwrap())
    );
    req.messages = vec![
        ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(first.content),
        },
        ChatMessage {
            role: Role::Tool,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "call_1".into(),
                content: "file content".into(),
                is_error: false,
            }]),
        },
    ];
    let second = tokio::time::timeout(Duration::from_secs(5), provider.complete(req))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.id, "resp_1");
    assert!(matches!(&second.content[0], ContentPart::Text { text } if text == "Done"));
    server.await.unwrap();
}

#[tokio::test]
async fn native_transport_does_not_surface_provider_error_bodies() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            headers.push(byte[0]);
            assert!(headers.len() < 16384);
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
        let len: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(len < 65536);
        socket.read_exact(&mut vec![0; len]).await.unwrap();
        socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 18\r\nConnection: close\r\n\r\nechoed-secret-body").await.unwrap();
    });
    let provider = temm1e_providers::OpenAICompatProvider::new("fixture".into())
        .with_name("openai")
        .with_base_url(format!("http://{address}"));
    let mut req = request();
    req.model = "gpt-6-astra".into();
    let error = tokio::time::timeout(Duration::from_secs(5), provider.complete(req))
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("401"));
    assert!(!error.to_string().contains("echoed-secret"));
    server.await.unwrap();
}
