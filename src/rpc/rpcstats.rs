use prometheus::{HistogramVec, IntGauge};

pub struct RPCStats {
    pub latency: HistogramVec,
    pub subscriptions: IntGauge,
}
