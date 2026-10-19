/// 本 crate 的错误。RPC 错误只保留文字描述：上层只需要决定重试或切换节点。
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("config: {0}")]
    Config(String),
    /// 节点返回的数据自相矛盾（例如日志缺字段、同一区块号两个哈希），本轮放弃，下一轮重试。
    #[error("inconsistent chain data: {0}")]
    Inconsistent(String),
}

impl From<alloy::transports::TransportError> for ChainError {
    fn from(e: alloy::transports::TransportError) -> Self {
        Self::Rpc(e.to_string())
    }
}
