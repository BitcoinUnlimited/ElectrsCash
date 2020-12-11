use crate::wstcp::server::ProxyServer;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;

pub mod channel;
pub mod frame;
pub mod opcode;
pub mod server;
pub mod util;

pub fn start_ws_proxy(bind_addr: SocketAddr, rpc_addr: SocketAddr) {
    let forward_addr = if rpc_addr.ip().is_unspecified() {
        // RPC bind address is 0.0.0.0, so we can't forward to that.
        // Use localhost.
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), rpc_addr.port())
    } else {
        rpc_addr
    };

    async_std::task::block_on(async {
        let proxy = ProxyServer::new(bind_addr, forward_addr)
            .await
            .unwrap_or_else(|e| panic!("{}", e));
        info!("WebSocket initalized");
        proxy.run_accept_loop().await.expect("WebSocket error");
    });
    info!("WebSocket closed")
}
