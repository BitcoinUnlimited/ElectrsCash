use crate::errors::*;
use crate::metrics::Metrics;

use prometheus::{IntCounter, IntGauge};

use std::convert::TryInto;
use std::net::IpAddr;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use std::collections::hash_map::Entry;
use std::collections::HashMap;

struct ConnectionMetrics {
    connections: IntGauge,
    connections_rejected_global: IntCounter,
    connections_rejected_prefix: IntCounter,
    connections_total: IntCounter,
}

pub struct GlobalLimits {
    /// Maximum number of connections we accept in total.
    max_connections_total: i32,

    /// Max connections from IP's sharing the first two octests (subnet mask
    /// 255.255.0.0 for ipv4)
    max_connections_shared_prefix: u32,

    /// Current total connections
    total_connections: AtomicI32,

    /// Current connections by octet prefix
    total_prefixed_connections: Mutex<HashMap<[u8; 2], u32>>,

    metrics: ConnectionMetrics,
}

fn get_prefix(addr: &IpAddr) -> [u8; 2] {
    match addr {
        IpAddr::V4(ipv4) => ipv4.octets()[..2].try_into().unwrap(),
        IpAddr::V6(ipv6) => ipv6.octets()[..2].try_into().unwrap(),
    }
}

impl GlobalLimits {
    pub fn new(
        max_connections_total: u32,
        max_connections_shared_prefix: u32,
        metric: &Metrics,
    ) -> GlobalLimits {
        GlobalLimits {
            max_connections_total: max_connections_total as i32,
            max_connections_shared_prefix,
            total_connections: AtomicI32::new(0),
            total_prefixed_connections: Mutex::new(HashMap::new()),
            metrics: ConnectionMetrics {
                connections: metric.gauge_int(prometheus::Opts::new(
                    "electrscash_rpc_connections",
                    "# of RPC connections",
                )),
                connections_rejected_global: metric.counter_int(prometheus::Opts::new(
                    "electrscash_rpc_connections_rejected_global",
                    "# of rejected RPC connections due to global slot limits",
                )),
                connections_rejected_prefix: metric.counter_int(prometheus::Opts::new(
                    "electrscash_rpc_connections_rejected_prefix",
                    "# of rejected RPC connections due to prefix slot limits",
                )),
                connections_total: metric.counter_int(prometheus::Opts::new(
                    "electrscash_rpc_connections_total",
                    "# of RPC connections since server start",
                )),
            },
        }
    }

    /// Increase connection count. Fails if maximum number of connections has
    /// been reached. Returns the new connection count.
    pub fn inc_connection(&self, addr: &IpAddr) -> Result<(u32, u32)> {
        self.metrics.connections_total.inc();
        let mut prefix_table = self.total_prefixed_connections.lock().unwrap();

        let prefix_count = match prefix_table.entry(get_prefix(addr)) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => v.insert(0),
        };

        if *prefix_count >= self.max_connections_shared_prefix {
            self.metrics.connections_rejected_prefix.inc();
            bail!(format!(
                "Maximum connection limit of {} reached for IP prefix {:?}.",
                self.max_connections_shared_prefix,
                get_prefix(addr)
            ))
        }

        // Check and update total connection limit.
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
            self.metrics.connections_rejected_global.inc();
            bail!(format!(
                "Maximum connection limit of {} reached.",
                self.max_connections_total
            ))
        };

        // All checks done, we can bump the prefix count now.
        *prefix_count += 1;

        // fetch_update fetches the *previous* value, that we succesfully bumped by one.
        let c = c.unwrap() + 1;
        self.metrics.connections.set(c as i64);
        Ok((c as u32, *prefix_count as u32))
    }

    /// Decreases connection count.
    pub fn dec_connection(&self, addr: &IpAddr) -> Result<(u32, u32)> {
        let mut prefix_table = self.total_prefixed_connections.lock().unwrap();
        let prefix_count = match prefix_table.get_mut(&get_prefix(addr)) {
            Some(count) => {
                *count -= 1;
                *count
            }
            None => {
                warn!("IP not found in prefix table");
                0
            }
        };
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
        // fetch_update fetches the *previous* value, that we succesfully decreased by one.
        let c = c.unwrap() - 1;
        self.metrics.connections.set(c as i64);
        Ok((c as u32, prefix_count))
    }

    /// connection limits as a tuple
    pub fn connection_limits(&self) -> (u32, u32) {
        (
            self.max_connections_total as u32,
            self.max_connections_shared_prefix,
        )
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

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_ip_shared_prefix() {
        let metrics = Metrics::dummy();

        let prefix_limit = 2;
        let limits = GlobalLimits::new(100, prefix_limit, &metrics);

        // Set of 3 ips that share the same two-octest prefix
        let ipv4_addr1 = Ipv4Addr::new(1, 2, 0, 4);
        let ipv4_addr2 = Ipv4Addr::new(1, 2, 100, 5);
        let ipv4_addr3 = Ipv4Addr::new(1, 2, 254, 6);

        let ipv6_addr1 = Ipv6Addr::new(1, 2, 1, 0, 0, 0, 0, 0);
        let ipv6_addr2 = Ipv6Addr::new(1, 2, 2, 0, 0, 0, 0, 0);
        let ipv6_addr3 = Ipv6Addr::new(1, 2, 3, 0, 0, 0, 0, 0);

        // Different prefix
        let ipv4_addr4 = Ipv4Addr::new(1, 3, 0, 4);
        let ipv6_addr4 = Ipv6Addr::new(0xf00d, 2, 1, 0, 0, 0, 0, 0);

        // Ipv4
        //
        assert_eq!(limits.inc_connection(&ipv4_addr1.into()).unwrap(), (1, 1));
        assert_eq!(limits.inc_connection(&ipv4_addr2.into()).unwrap(), (2, 2));
        assert!(limits.inc_connection(&ipv4_addr3.into()).is_err());
        assert_eq!(limits.inc_connection(&ipv4_addr4.into()).unwrap(), (3, 1));

        // Disconnecting addr1 should allow for addr3 to connect
        assert_eq!(limits.dec_connection(&ipv4_addr1.into()).unwrap(), (2, 1));
        assert_eq!(limits.inc_connection(&ipv4_addr3.into()).unwrap(), (3, 2));

        // Ipv6
        //
        assert_eq!(limits.inc_connection(&ipv6_addr1.into()).unwrap(), (4, 1));
        assert_eq!(limits.inc_connection(&ipv6_addr2.into()).unwrap(), (5, 2));
        assert!(limits.inc_connection(&ipv6_addr3.into()).is_err());
        assert_eq!(limits.inc_connection(&ipv6_addr4.into()).unwrap(), (6, 1));

        // Disconnecting addr1 should allow for addr3 to connect
        assert_eq!(limits.dec_connection(&ipv6_addr1.into()).unwrap(), (5, 1));
        assert_eq!(limits.inc_connection(&ipv6_addr3.into()).unwrap(), (6, 2));
    }
}
