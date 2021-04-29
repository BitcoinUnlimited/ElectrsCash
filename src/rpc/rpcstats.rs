use prometheus::{HistogramVec, IntGauge};

pub struct RpcStats {
    pub latency: HistogramVec,
    pub subscriptions: IntGauge,
}
