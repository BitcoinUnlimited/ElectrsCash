use crate::errors::*;
use crate::metrics::Metrics;

use prometheus::IntGauge;

use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

pub struct GlobalLimits {
    /// Maximum number of connections we accept in total.
    pub max_connections_total: i32,

    /// Current total connections
    total_connections: AtomicI32,

    metric_connections: IntGauge,
}

impl GlobalLimits {
    pub fn new(max_connections_total: u32, metric: &Metrics) -> GlobalLimits {
        GlobalLimits {
            max_connections_total: max_connections_total as i32,
            total_connections: AtomicI32::new(0),
            metric_connections: metric.gauge_int(prometheus::Opts::new(
                "electrscash_rpc_connections",
                "# of RPC connections",
            )),
        }
    }

    /// Increase connection count. Fails if maximum number of connections has
    /// been reached. Returns the new connection count.
    pub fn inc_connection(&self) -> Result<u32> {
        let c =
            self.total_connections
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    if current < self.max_connections_total {
                        Some(current + 1)
                    } else {
                        None
                    }
                });

        if c.is_err() {
            bail!(format!(
                "Maximum connection limit of {} reached.",
                self.max_connections_total
            ))
        };
        let c = c.unwrap();
        // fetch_update fetches the *previous* value, that we succesfully bumped by one.
        let c = c + 1;
        self.metric_connections.set(c as i64);
        Ok(c as u32)
    }

    /// Decreases connection count.
    pub fn dec_connection(&self) -> Result<u32> {
        let c =
            self.total_connections
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    if current <= 0 {
                        None
                    } else {
                        Some(current - 1)
                    }
                });
        if c.is_err() {
            bail!("Cannot decrease connection counter. Already at zero.");
        }
        let c = c.unwrap();
        // fetch_update fetches the *previous* value, that we succesfully decreased by one.
        let c = c - 1;
        self.metric_connections.set(c as i64);
        Ok(c as u32)
    }

    /// Get maximum connections allowed
    pub fn max_connections(&self) -> u32 {
        self.max_connections_total as u32
    }
}

/// DoS limits per connection
#[derive(Clone, Copy)]
pub struct ConnectionLimits {
    /// Maximum execution time per RPC call (in seconds)
    pub rpc_timeout: u16,

    /// Maximum number of scripthash subscriptions per connection
    pub max_subscriptions: u32,

    /// Maximum number of bytes used to alias scripthash subscriptions.
    /// (scripthash aliased by bitcoin cash address)
    pub max_alias_bytes: u32,
}

/// Limits specific for a connecting peer.
impl ConnectionLimits {
    pub fn new(rpc_timeout: u16, max_subscriptions: u32, max_alias_bytes: u32) -> ConnectionLimits {
        ConnectionLimits {
            rpc_timeout,
            max_subscriptions,
            max_alias_bytes,
        }
    }

    pub fn check_subscriptions(&self, num_subscriptions: u32) -> Result<()> {
        if num_subscriptions <= self.max_subscriptions as u32 {
            return Ok(());
        }

        Err(rpc_invalid_request(format!(
            "Scripthash subscriptions limit reached (max {})",
            self.max_subscriptions
        ))
        .into())
    }

    pub fn check_alias_usage(&self, bytes_used: usize) -> Result<()> {
        if bytes_used <= self.max_alias_bytes as usize {
            return Ok(());
        }

        Err(rpc_invalid_request(format!(
            "Address/alias subscriptions limit reached (max {} bytes) \
            Use scripthash subscriptions for more subscriptions or increase this limit.",
            self.max_alias_bytes
        ))
        .into())
    }
}
