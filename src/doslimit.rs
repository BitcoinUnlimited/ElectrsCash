use crate::errors::*;

/// DoS limits per connection
#[derive(Clone, Copy)]
pub struct ConnectionLimits {
    /// Maximum execution time per RPC call (in seconds)
    pub rpc_timeout: u16,

    /// Maximum number of scripthash subscriptions per connection
    pub max_subscriptions: u32,

    /// TODO: Maximum number of bytes used to alias scripthash subscriptions.
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

    pub fn check_subscriptions(&self, num_subscriptions: usize) -> Result<()> {
        if num_subscriptions <= self.max_subscriptions as usize {
            return Ok(());
        }

        Err(rpc_invalid_request(format!(
            "Scripthash subscriptions limit reached (max {})",
            self.max_subscriptions
        ))
        .into())
    }
}
