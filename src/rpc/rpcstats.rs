use crate::metrics::{Gauge, HistogramVec};

pub struct RPCStats {
    pub latency: HistogramVec,
    pub subscriptions: Gauge,
}
