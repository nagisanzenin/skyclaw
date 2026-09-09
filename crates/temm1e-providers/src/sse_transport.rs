//! Shared bounded byte transport; protocol state decides when a response is complete.
use futures::{stream::BoxStream, StreamExt};
use temm1e_core::{
    sse::{SseDecoder, SseEvent},
    types::{error::Temm1eError, message::StreamChunk},
};
pub(crate) trait Protocol: Send + 'static {
    fn accept(&mut self, event: SseEvent) -> Result<(), Temm1eError>;
    fn pop(&mut self) -> Option<StreamChunk>;
    fn done(&self) -> bool;
    fn finish(&self) -> Result<(), Temm1eError>;
    fn end(&mut self) -> Result<(), Temm1eError> {
        self.finish()
    }
}
pub(crate) fn stream<D: Protocol>(
    response: reqwest::Response,
    decoder: D,
) -> BoxStream<'static, Result<StreamChunk, Temm1eError>> {
    let state = (
        Box::pin(response.bytes_stream()),
        SseDecoder::new(2 * 1024 * 1024),
        decoder,
        0usize,
        false,
    );
    Box::pin(futures::stream::unfold(
        state,
        |(mut bytes, mut sse, mut decoder, mut total, mut ended)| async move {
            loop {
                if let Some(chunk) = decoder.pop() {
                    return Some((Ok(chunk), (bytes, sse, decoder, total, ended)));
                }
                if ended || decoder.done() {
                    return None;
                }
                let result = match bytes.next().await {
                    Some(Ok(chunk)) => {
                        total = total.saturating_add(chunk.len());
                        if total > 64 * 1024 * 1024 {
                            Err(Temm1eError::Provider(
                                "Stream wire response exceeds 64 MiB limit".into(),
                            ))
                        } else {
                            let mut result = Ok(());
                            for byte in chunk {
                                match sse.push(byte) {
                                    Ok(Some(event)) => {
                                        if let Err(e) = decoder.accept(event) {
                                            result = Err(e);
                                            break;
                                        }
                                        if decoder.done() {
                                            break;
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        result = Err(e);
                                        break;
                                    }
                                }
                            }
                            result
                        }
                    }
                    Some(Err(e)) => Err(Temm1eError::Provider(format!(
                        "Stream read failed: {}",
                        e.without_url()
                    ))),
                    None => {
                        ended = true;
                        sse.finish().and_then(|_| decoder.end())
                    }
                };
                if let Err(e) = result {
                    // Never expose queued output after a failed batch. EOF can
                    // enqueue final chunks only when validation succeeded.
                    while decoder.pop().is_some() {}
                    ended = true;
                    return Some((Err(e), (bytes, sse, decoder, total, ended)));
                }
            }
        },
    ))
}
