use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use serde_json::json;

use super::{
    error::ProxyError,
    metrics::estimate_tokens_from_char_count,
    response::{PreparedResponse, StreamCompletion},
    server::ProxyServerState,
};

pub struct ResponseHandler;

impl ResponseHandler {
    pub async fn finish_buffered(
        state: &ProxyServerState,
        response_result: Result<PreparedResponse, ProxyError>,
        status: reqwest::StatusCode,
    ) -> Response {
        match response_result {
            Ok(response) => {
                let PreparedResponse {
                    response,
                    estimated_output_tokens,
                    ..
                } = response;
                state
                    .record_estimated_output_tokens(estimated_output_tokens)
                    .await;
                if status.is_success() {
                    state.record_request_success().await;
                } else {
                    state.record_upstream_failure(status).await;
                }
                response
            }
            Err(error) => {
                state.record_request_error(&error).await;
                proxy_error_response(error)
            }
        }
    }

    pub async fn finish_streaming(
        state: &ProxyServerState,
        response_result: Result<PreparedResponse, ProxyError>,
        status: reqwest::StatusCode,
    ) -> Response {
        match response_result {
            Ok(response) => track_streaming_response(state.clone(), response, status),
            Err(error) => {
                state.record_request_error(&error).await;
                proxy_error_response(error)
            }
        }
    }
}

fn track_streaming_response(
    state: ProxyServerState,
    response: PreparedResponse,
    status: reqwest::StatusCode,
) -> Response {
    let (parts, body) = response.response.into_parts();
    let mut recorder = StreamingOutcomeRecorder::new(state, response.stream_completion, status);
    let tracked_stream = async_stream::stream! {
        let mut stream = body.into_data_stream();

        while let Some(next) = stream.next().await {
            match next {
                Ok(chunk) => {
                    recorder.record_chunk(&chunk);
                    yield Ok(chunk)
                }
                Err(error) => {
                    recorder.finish();
                    yield Err(std::io::Error::other(error));
                    return;
                }
            }
        }

        recorder.finish();
    };

    Response::from_parts(parts, Body::from_stream(tracked_stream))
}

struct StreamingOutcomeRecorder {
    state: ProxyServerState,
    stream_completion: Option<StreamCompletion>,
    status: reqwest::StatusCode,
    output_char_count: u64,
    finished: bool,
}

impl StreamingOutcomeRecorder {
    fn new(
        state: ProxyServerState,
        stream_completion: Option<StreamCompletion>,
        status: reqwest::StatusCode,
    ) -> Self {
        Self {
            state,
            stream_completion,
            status,
            output_char_count: 0,
            finished: false,
        }
    }

    fn record_chunk(&mut self, chunk: &bytes::Bytes) {
        self.output_char_count = self
            .output_char_count
            .saturating_add(String::from_utf8_lossy(chunk).chars().count() as u64);
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;

        let state = self.state.clone();
        let estimated_output_tokens = estimate_tokens_from_char_count(self.output_char_count);
        if !self.status.is_success() {
            let status = self.status;
            tokio::spawn(async move {
                state
                    .record_estimated_output_tokens(estimated_output_tokens)
                    .await;
                state.record_upstream_failure(status).await;
            });
            return;
        }

        match self
            .stream_completion
            .as_ref()
            .and_then(StreamCompletion::outcome)
        {
            Some(Err(message)) => {
                tokio::spawn(async move {
                    state
                        .record_estimated_output_tokens(estimated_output_tokens)
                        .await;
                    state.record_request_error_message(message).await;
                });
            }
            Some(Ok(())) => {
                tokio::spawn(async move {
                    state
                        .record_estimated_output_tokens(estimated_output_tokens)
                        .await;
                    state.record_request_success().await;
                });
            }
            None => {
                tokio::spawn(async move {
                    state
                        .record_estimated_output_tokens(estimated_output_tokens)
                        .await;
                    state
                        .record_request_error_message(
                            "stream terminated before completion".to_string(),
                        )
                        .await;
                });
            }
        }
    }
}

impl Drop for StreamingOutcomeRecorder {
    fn drop(&mut self) {
        self.finish();
    }
}

pub fn proxy_error_response(error: ProxyError) -> Response {
    match error {
        ProxyError::ConfigError(message) | ProxyError::AuthError(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
        }
        ProxyError::RequestFailed(message) | ProxyError::TransformError(message) => {
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": message }))).into_response()
        }
        ProxyError::UpstreamError { status, body } => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, Json(json!({ "error": body }))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use axum::{
        body::{to_bytes, Body},
        response::Response,
    };
    use bytes::Bytes;
    use tokio::sync::RwLock;

    use crate::{database::Database, proxy::types::ProxyConfig};

    use super::*;

    fn test_state() -> ProxyServerState {
        ProxyServerState {
            db: Arc::new(Database::memory().expect("memory db")),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            status: Arc::new(RwLock::new(crate::proxy::types::ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn settle_tasks() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn buffered_failures_still_accumulate_output_tokens() {
        let state = test_state();
        state.record_request_start().await;

        let response = PreparedResponse {
            response: Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("upstream failure payload"))
                .expect("response"),
            stream_completion: None,
            estimated_output_tokens: 9,
        };

        let _ = ResponseHandler::finish_buffered(
            &state,
            Ok(response),
            reqwest::StatusCode::BAD_GATEWAY,
        )
        .await;

        let snapshot = state.snapshot_status().await;
        assert_eq!(snapshot.failed_requests, 1);
        assert_eq!(snapshot.estimated_output_tokens_total, 9);
    }

    #[tokio::test]
    async fn interrupted_streams_keep_partial_output_estimate() {
        let state = test_state();
        state.record_request_start().await;

        let stream = async_stream::stream! {
            yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"partial output"));
            yield Err::<Bytes, std::io::Error>(std::io::Error::other("boom"));
        };
        let response = PreparedResponse {
            response: Response::builder()
                .status(StatusCode::OK)
                .body(Body::from_stream(stream))
                .expect("response"),
            stream_completion: None,
            estimated_output_tokens: 0,
        };

        let response =
            ResponseHandler::finish_streaming(&state, Ok(response), reqwest::StatusCode::OK).await;
        let _ = to_bytes(response.into_body(), usize::MAX).await;
        settle_tasks().await;

        let snapshot = state.snapshot_status().await;
        assert_eq!(snapshot.failed_requests, 1);
        assert!(snapshot.estimated_output_tokens_total > 0);
    }

    #[tokio::test]
    async fn non_success_streams_accumulate_output_tokens_after_body_drains() {
        let state = test_state();
        state.record_request_start().await;

        let response = PreparedResponse {
            response: Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("bad request payload"))
                .expect("response"),
            stream_completion: None,
            estimated_output_tokens: 0,
        };

        let response = ResponseHandler::finish_streaming(
            &state,
            Ok(response),
            reqwest::StatusCode::BAD_REQUEST,
        )
        .await;
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        settle_tasks().await;

        let snapshot = state.snapshot_status().await;
        assert_eq!(snapshot.failed_requests, 1);
        assert!(snapshot.estimated_output_tokens_total > 0);
    }
}
