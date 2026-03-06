//! 流式健康检查服务
//!
//! 使用流式 API 进行快速健康检查，只需接收首个 chunk 即判定成功。

mod provider_extract;
mod request_builders;
mod service;
#[cfg(test)]
mod tests;
mod types;

pub use service::StreamCheckService;
pub use types::{HealthStatus, StreamCheckConfig, StreamCheckResult};
