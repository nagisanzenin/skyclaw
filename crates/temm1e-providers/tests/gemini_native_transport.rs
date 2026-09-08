use futures::StreamExt;
use serde_json::{json, Value};
use temm1e_core::{types::message::*, Provider};
use temm1e_providers::GeminiProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn request() -> CompletionRequest {
    CompletionRequest {
        model: "gemini-3.8-flash".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text("read a file".into()),
        }],
        tools: vec![ToolDefinition {
            name: "read_file".into(),
            description: "fixture".into(),
            parameters: json!({"type":"object"}),
        }],
        max_tokens: Some(256),
        temperature: Some(1.0),
        system: Some("fixture system".into()),
        system_volatile: None,
    }
}
async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut b = [0];
        socket.read_exact(&mut b).await.unwrap();
        header.push(b[0]);
        assert!(header.len() < 65536);
    }
    let header = String::from_utf8(header).unwrap().to_lowercase();
    let length: usize = header
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(length < 65536);
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    (header, serde_json::from_slice(&body).unwrap())
}
fn event(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}

#[tokio::test]
async fn real_sse_is_incremental_and_native_function_history_survives_restore() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (release, wait) = tokio::sync::oneshot::channel();
    let native = json!([{"text":"Hello 🌿"},{"text":"private fixture thought","thought":true,"thoughtSignature":"text-signature"},{"functionCall":{"id":"provider-call","name":"read_file","args":{"path":"a.txt"}},"thoughtSignature":"call-signature"}]);
    let expected = native.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (head, body) = read_request(&mut socket).await;
        assert!(
            head.starts_with("post /v1beta/models/gemini-3.8-flash:streamgeneratecontent?alt=sse ")
        );
        assert!(head.contains("x-goog-api-key: fixture-key"));
        assert!(!head.lines().next().unwrap().contains("fixture-key"));
        assert_eq!(body["generationConfig"]["candidateCount"], 1);
        let first = event(
            json!({"responseId":"response-1","candidates":[{"index":0,"content":{"role":"model","parts":[expected[0].clone()]}}]}),
        );
        let rest = event(
            json!({"responseId":"response-1","candidates":[{"index":0,"content":{"role":"model","parts":[expected[1].clone(),expected[2].clone()]},"finishReason":"STOP"}]}),
        ) + &event(
            json!({"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":5,"thoughtsTokenCount":7,"totalTokenCount":32,"cachedContentTokenCount":10}}),
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",first.len()+rest.len()).as_bytes()).await.unwrap();
        // Deliberately fragment the UTF-8/event bytes, then hold the rest until
        // the client proves it received the first public delta.
        for byte in first.as_bytes() {
            socket.write_all(&[*byte]).await.unwrap();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
        socket.write_all(rest.as_bytes()).await.unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let (head, body) = read_request(&mut socket).await;
        assert!(head.starts_with("post /v1beta/models/gemini-3.8-flash:generatecontent "));
        assert_eq!(body["contents"][1]["parts"], expected);
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"],
            json!({"id":"provider-call","name":"read_file","response":{"result":"file content"}})
        );
        let reply=json!({"responseId":"response-2","candidates":[{"content":{"role":"model","parts":[{"text":"done"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":30,"candidatesTokenCount":2,"thoughtsTokenCount":0,"totalTokenCount":32}}).to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let provider =
        GeminiProvider::new("fixture-key".into()).with_base_url(format!("http://{addr}/v1beta"));
    let mut req = request();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured = seen.clone();
    let release = std::sync::Mutex::new(Some(release));
    let observer = std::sync::Arc::new(move |text: &str| {
        captured.lock().unwrap().push_str(text);
        if let Some(release) = release.lock().unwrap().take() {
            release.send(()).unwrap();
        }
    });
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        provider.complete_with_observer(req.clone(), observer),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), "Hello 🌿");
    let parts = response.content;
    let usage = response.usage;
    assert_eq!(usage.output_tokens, 12);
    assert_eq!(usage.cache_read_tokens, Some(10));
    assert_eq!(usage.totals_reported, Some(true));
    let restored = serde_json::from_slice(&serde_json::to_vec(&parts).unwrap()).unwrap();
    req.messages.push(ChatMessage {
        role: Role::Assistant,
        content: MessageContent::Parts(restored),
    });
    // Agent history represents tool results in User messages as well as Tool.
    req.messages.push(ChatMessage {
        role: Role::User,
        content: MessageContent::Parts(vec![ContentPart::ToolResult {
            tool_use_id: "provider-call".into(),
            content: "file content".into(),
            is_error: false,
        }]),
    });
    let result = provider.complete(req).await.unwrap();
    assert!(result
        .content
        .iter()
        .any(|p| matches!(p,ContentPart::Text{text} if text=="done")));
    server.await.unwrap();
}

#[tokio::test]
async fn truncated_stream_never_exposes_pending_function() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let reply = event(
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","args":{}}}]}}]}),
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",reply.len()).as_bytes()).await.unwrap();
    });
    let provider =
        GeminiProvider::new("fixture".into()).with_base_url(format!("http://{addr}/v1beta"));
    let mut stream = provider.stream(request()).await.unwrap();
    assert!(stream.next().await.unwrap().is_err());
    assert!(stream.next().await.is_none());
    server.await.unwrap();
}
