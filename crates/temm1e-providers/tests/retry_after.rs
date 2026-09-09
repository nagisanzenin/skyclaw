use std::time::Duration;
use temm1e_core::{types::message::CompletionRequest, Provider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn long_retry_after_returns_without_retrying_or_waiting_for_all_backends() {
    for anthropic in [false, true] {
        for streaming in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 4096];
                    let n = socket.read(&mut bytes).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&bytes[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                    assert!(request.len() < 65_536);
                }
                socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 5\r\nConnection: close\r\n\r\nquota").await.unwrap();
                socket.shutdown().await.unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err(),
                    "must not issue an early retry"
                );
            });
            let url = format!("http://{address}");
            let provider: Box<dyn Provider> = if anthropic {
                Box::new(
                    temm1e_providers::AnthropicProvider::new("fixture".into()).with_base_url(url),
                )
            } else {
                Box::new(
                    temm1e_providers::OpenAICompatProvider::new("fixture".into())
                        .with_base_url(url),
                )
            };
            let request = CompletionRequest {
                model: "fixture".into(),
                messages: vec![],
                tools: vec![],
                max_tokens: Some(8),
                temperature: None,
                system: None,
                system_volatile: None,
            };
            let error = tokio::time::timeout(Duration::from_secs(2), async {
                if streaming {
                    provider.stream(request).await.err().unwrap()
                } else {
                    provider.complete(request).await.err().unwrap()
                }
            })
            .await
            .expect("long quota waits must return promptly");
            assert!(error.to_string().contains("120"), "{error}");
            server.await.unwrap();
        }
    }
}
