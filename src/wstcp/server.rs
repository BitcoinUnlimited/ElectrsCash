use crate::errors::*;
use crate::wstcp::channel::ProxyChannel;
use async_std::net::TcpListener;
use std::net::SocketAddr;

/// WebSocket to TCP proxy server.
#[derive(Debug)]
pub struct ProxyServer {
    proxy_addr: SocketAddr,
    real_server_addr: SocketAddr,
    listener: TcpListener,
}
impl ProxyServer {
    /// Makes a new `ProxyServer` instance.
    pub async fn new(proxy_addr: SocketAddr, real_server_addr: SocketAddr) -> Result<Self> {
        info!("Starting a WebSocket server on {}", proxy_addr.to_string());
        trace!("WebSocket proxy to {}", real_server_addr.to_string());
        let listener = TcpListener::bind(proxy_addr)
            .await
            .expect("failed to bind websocket server");
        Ok(ProxyServer {
            proxy_addr,
            real_server_addr,
            listener,
        })
    }

    pub async fn run_accept_loop(&self) -> Result<()> {
        loop {
            let stream = self.listener.accept().await;

            match stream {
                Ok((stream, addr)) => {
                    debug!("New connection: {}", addr);

                    let channel = ProxyChannel::new(stream, self.real_server_addr);
                    async_std::task::spawn(async move {
                        match channel.await {
                            Err(e) => {
                                warn!("A proxy channel aborted: {}", e);
                            }
                            Ok(()) => {
                                info!("A proxy channel terminated normally");
                            }
                        }
                    });
                }
                Err(e) => {
                    trace!("Incoming connection error {}", e);
                }
            }
        }
    }
}
