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
