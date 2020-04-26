use bitcoin::blockdata::transaction::Transaction;
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use error_chain::ChainedError;
use serde_json::{from_str, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::def::PROTOCOL_VERSION_MAX;
use crate::errors::*;
use crate::metrics::{HistogramOpts, HistogramVec, Metrics};
use crate::query::Query;
use crate::rpc::blockchain::BlockchainRPC;
use crate::rpc::parseutil::usize_from_value;
use crate::rpc::server::{
    server_add_peer, server_banner, server_donation_address, server_features,
    server_peers_subscribe, server_version,
};
use crate::scripthash::{compute_script_hash, FullHash};
use crate::timeout::TimeoutTrigger;
use crate::util::{spawn_thread, Channel, HeaderEntry, SyncChannel};

pub mod blockchain;
pub mod parseutil;
pub mod scripthash;
pub mod server;

fn get_output_scripthash(txn: &Transaction, n: Option<usize>) -> Vec<FullHash> {
    if let Some(out) = n {
        vec![compute_script_hash(&txn.output[out].script_pubkey[..])]
    } else {
        txn.output
            .iter()
            .map(|o| compute_script_hash(&o.script_pubkey[..]))
            .collect()
    }
}

struct Connection {
    query: Arc<Query>,
    stream: TcpStream,
    addr: SocketAddr,
    chan: SyncChannel<Message>,
    stats: Arc<Stats>,
    rpc_timeout: u16,
    blockchainrpc: BlockchainRPC,
}

impl Connection {
    pub fn new(
        query: Arc<Query>,
        metrics: Arc<Metrics>,
        stream: TcpStream,
        addr: SocketAddr,
        stats: Arc<Stats>,
        relayfee: f64,
        rpc_timeout: u16,
        buffer_size: usize,
    ) -> Connection {
        Connection {
            query: query.clone(),
            stream,
            addr,
            chan: SyncChannel::new(buffer_size),
            stats: stats.clone(),
            rpc_timeout,
            blockchainrpc: BlockchainRPC::new(query, metrics, relayfee, rpc_timeout),
        }
    }

    fn mempool_get_fee_histogram(&self) -> Result<Value> {
        Ok(json!(self.query.get_fee_histogram()))
    }

    fn cashaccount_query_name(&self, params: &[Value]) -> Result<Value> {
        let name = params.get(0).chain_err(|| "missing name")?;
        let name = name.as_str().chain_err(|| "bad accountname")?;
        let height = usize_from_value(params.get(1), "height")?;

        self.query.get_cashaccount_txs(name, height as u32)
    }

    fn handle_command(&mut self, method: &str, params: &[Value], id: &Value) -> Result<Value> {
        let timer = self
            .stats
            .latency
            .with_label_values(&[method])
            .start_timer();
        let timeout = TimeoutTrigger::new(Duration::from_secs(self.rpc_timeout as u64));
        let result = match method {
            "blockchain.address.get_balance" => {
                self.blockchainrpc.address_get_balance(&params, &timeout)
            }
            "blockchain.address.get_first_use" => self.blockchainrpc.address_get_first_use(&params),
            "blockchain.address.get_history" => {
                self.blockchainrpc.address_get_history(&params, &timeout)
            }
            "blockchain.address.listunspent" => {
                self.blockchainrpc.address_listunspent(&params, &timeout)
            }
            "blockchain.block.header" => self.blockchainrpc.block_header(&params),
            "blockchain.block.headers" => self.blockchainrpc.block_headers(&params),
            "blockchain.estimatefee" => self.blockchainrpc.estimatefee(&params),
            "blockchain.headers.subscribe" => self.blockchainrpc.headers_subscribe(),
            "blockchain.relayfee" => self.blockchainrpc.relayfee(),
            "blockchain.scripthash.get_balance" => {
                self.blockchainrpc.scripthash_get_balance(&params, &timeout)
            }
            "blockchain.scripthash.get_first_use" => {
                self.blockchainrpc.scripthash_get_first_use(&params)
            }
            "blockchain.scripthash.get_history" => {
                self.blockchainrpc.scripthash_get_history(&params, &timeout)
            }
            "blockchain.scripthash.listunspent" => {
                self.blockchainrpc.scripthash_listunspent(&params, &timeout)
            }
            "blockchain.scripthash.subscribe" => {
                self.blockchainrpc.scripthash_subscribe(&params, &timeout)
            }
            "blockchain.scripthash.unsubscribe" => {
                self.blockchainrpc.scripthash_unsubscribe(&params)
            }
            "blockchain.transaction.broadcast" => self.blockchainrpc.transaction_broadcast(&params),
            "blockchain.transaction.get" => self.blockchainrpc.transaction_get(&params),
            "blockchain.transaction.get_merkle" => {
                self.blockchainrpc.transaction_get_merkle(&params)
            }
            "blockchain.transaction.id_from_pos" => {
                self.blockchainrpc.transaction_id_from_pos(&params)
            }
            "mempool.get_fee_histogram" => self.mempool_get_fee_histogram(),
            "server.add_peer" => server_add_peer(),
            "server.banner" => server_banner(&self.query),
            "server.donation_address" => server_donation_address(),
            "server.features" => server_features(&self.query),
            "server.peers.subscribe" => server_peers_subscribe(),
            "server.ping" => Ok(Value::Null),
            "server.version" => server_version(&params),
            "cashaccount.query.name" => self.cashaccount_query_name(&params),
            &_ => Err(ErrorKind::RpcError(
                RpcErrorCode::MethodNotFound,
                format!("unknown method {}", method),
            )
            .into()),
        };
        timer.observe_duration();
        // TODO: return application errors should be sent to the client
        Ok(if let Err(e) = result {
            match *e.kind() {
                ErrorKind::RpcError(ref code, _) => {
                    // Use (at most) two errors from the error chain to produce
                    // an error descrption.
                    let errmsgs: Vec<String> = e.iter().take(2).map(|x| x.to_string()).collect();
                    let errmsgs = errmsgs.join("; ");
                    json!({"jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": *code as i32,
                        "message": errmsgs,
                    }})
                }
                _ => {
                    warn!(
                        "rpc #{} {} {:?} failed: {}",
                        id,
                        method,
                        params,
                        e.display_chain()
                    );

                    json!({"jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": RpcErrorCode::InternalError as i32,
                        "message": e.to_string()
                    }})
                }
            }
        } else {
            json!({"jsonrpc": "2.0", "id": id, "result": result.unwrap() })
        })
    }

    pub fn send_values(&mut self, values: &[Value]) -> Result<()> {
        for value in values {
            let line = value.to_string() + "\n";
            if let Err(e) = self.stream.write_all(line.as_bytes()) {
                let truncated: String = line.chars().take(80).collect();
                return Err(e).chain_err(|| format!("failed to send {}", truncated));
            }
        }
        Ok(())
    }

    fn handle_replies(&mut self) -> Result<()> {
        let empty_params = json!([]);
        loop {
            let msg = self.chan.receiver().recv().chain_err(|| "channel closed")?;
            match msg {
                Message::Request(line) => {
                    trace!("RPC {:?}", line);
                    let cmd: Value = from_str(&line).chain_err(|| "invalid JSON format")?;
                    let reply = match (
                        cmd.get("method"),
                        cmd.get("params").unwrap_or_else(|| &empty_params),
                        cmd.get("id"),
                    ) {
                        (
                            Some(&Value::String(ref method)),
                            &Value::Array(ref params),
                            Some(ref id),
                        ) => self.handle_command(method, params, id)?,
                        _ => bail!("invalid command: {}", cmd),
                    };
                    self.send_values(&[reply])?
                }
                Message::ScriptHashChange(hash) => {
                    let notification = self.blockchainrpc.on_scripthash_change(hash)?;
                    if let Some(n) = notification {
                        self.send_values(&[n])?;
                    }
                }
                Message::ChainTipChange(tip) => {
                    let notification = self.blockchainrpc.on_chaintip_change(tip)?;
                    if let Some(n) = notification {
                        self.send_values(&[n])?;
                    }
                }
                Message::Done => return Ok(()),
            }
        }
    }

    fn handle_requests(mut reader: BufReader<TcpStream>, tx: SyncSender<Message>) -> Result<()> {
        loop {
            let mut line = Vec::<u8>::new();
            reader
                .read_until(b'\n', &mut line)
                .chain_err(|| "failed to read a request")?;
            if line.is_empty() {
                tx.send(Message::Done).chain_err(|| "channel closed")?;
                return Ok(());
            } else {
                if line.starts_with(&[22, 3, 1]) {
                    // (very) naive SSL handshake detection
                    let _ = tx.send(Message::Done);
                    bail!("invalid request - maybe SSL-encrypted data?: {:?}", line)
                }
                match String::from_utf8(line) {
                    Ok(req) => tx
                        .send(Message::Request(req))
                        .chain_err(|| "channel closed")?,
                    Err(err) => {
                        let _ = tx.send(Message::Done);
                        bail!("invalid UTF8: {}", err)
                    }
                }
            }
        }
    }

    pub fn run(mut self) {
        let reader = BufReader::new(self.stream.try_clone().expect("failed to clone TcpStream"));
        let tx = self.chan.sender();
        let child = spawn_thread("reader", || Connection::handle_requests(reader, tx));
        if let Err(e) = self.handle_replies() {
            debug!(
                "[{}] connection handling failed: {}",
                self.addr,
                e.display_chain().to_string()
            );
        }
        debug!("[{}] shutting down connection", self.addr);
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Err(err) = child.join().expect("receiver panicked") {
            error!("[{}] receiver failed: {}", self.addr, err);
        }
    }
}

#[derive(Debug)]
pub enum Message {
    Request(String),
    ScriptHashChange(FullHash),
    ChainTipChange(HeaderEntry),
    Done,
}

pub enum Notification {
    ScriptHashChange(FullHash),
    ChainTipChange(HeaderEntry),
    Exit,
}

pub struct RPC {
    notification: Sender<Notification>,
    server: Option<thread::JoinHandle<()>>, // so we can join the server while dropping this ojbect
    query: Arc<Query>,
}

struct Stats {
    latency: HistogramVec,
}

impl RPC {
    fn start_notifier(
        notification: Channel<Notification>,
        senders: Arc<Mutex<HashMap<i32, SyncSender<Message>>>>,
        acceptor: Sender<Option<(TcpStream, SocketAddr)>>,
    ) {
        spawn_thread("notification", move || {
            for msg in notification.receiver().iter() {
                let senders = senders.lock().unwrap();
                match msg {
                    Notification::ScriptHashChange(hash) => {
                        for (i, sender) in senders.iter() {
                            if let Err(e) = sender.try_send(Message::ScriptHashChange(hash)) {
                                debug!("failed to send ScriptHashChange to peer {}: {}", i, e);
                            }
                        }
                    }
                    Notification::ChainTipChange(hash) => {
                        for (i, sender) in senders.iter() {
                            if let Err(e) = sender.try_send(Message::ChainTipChange(hash.clone())) {
                                debug!("failed to send ChainTipChange to peer {}: {}", i, e);
                            }
                        }
                    }
                    Notification::Exit => acceptor.send(None).unwrap(),
                }
            }
        });
    }

    fn start_acceptor(addr: SocketAddr) -> Channel<Option<(TcpStream, SocketAddr)>> {
        let chan = Channel::unbounded();
        let acceptor = chan.sender();
        spawn_thread("acceptor", move || {
            let listener =
                TcpListener::bind(addr).unwrap_or_else(|e| panic!("bind({}) failed: {}", addr, e));
            info!(
                "Electrum RPC server running on {} (protocol {})",
                addr, PROTOCOL_VERSION_MAX
            );
            loop {
                let (stream, addr) = listener.accept().expect("accept failed");
                stream
                    .set_nonblocking(false)
                    .expect("failed to set connection as blocking");
                acceptor.send(Some((stream, addr))).expect("send failed");
            }
        });
        chan
    }

    pub fn start(
        addr: SocketAddr,
        query: Arc<Query>,
        metrics: Arc<Metrics>,
        relayfee: f64,
        rpc_timeout: u16,
        rpc_buffer_size: usize,
    ) -> RPC {
        let stats = Arc::new(Stats {
            latency: metrics.histogram_vec(
                HistogramOpts::new("electrscash_electrum_rpc", "Electrum RPC latency (seconds)"),
                &["method"],
            ),
        });
        let notification = Channel::unbounded();
        RPC {
            notification: notification.sender(),
            query: query.clone(),
            server: Some(spawn_thread("rpc", move || {
                let senders = Arc::new(Mutex::new(HashMap::<i32, SyncSender<Message>>::new()));
                let handles = Arc::new(Mutex::new(
                    HashMap::<i32, std::thread::JoinHandle<()>>::new(),
                ));

                let acceptor = RPC::start_acceptor(addr);
                RPC::start_notifier(notification, senders.clone(), acceptor.sender());

                let mut handle_count = 0;
                while let Some((stream, addr)) = acceptor.receiver().recv().unwrap() {
                    let handle_id = handle_count;
                    handle_count += 1;

                    // explicitely scope the shadowed variables for the new thread
                    let handle: thread::JoinHandle<()> = {
                        let query = Arc::clone(&query);
                        let senders = Arc::clone(&senders);
                        let stats = Arc::clone(&stats);
                        let handles = Arc::clone(&handles);
                        let metrics = Arc::clone(&metrics);

                        spawn_thread("peer", move || {
                            info!("[{}] connected peer #{}", addr, handle_id);
                            let conn = Connection::new(
                                query,
                                metrics,
                                stream,
                                addr,
                                stats,
                                relayfee,
                                rpc_timeout,
                                rpc_buffer_size,
                            );
                            senders
                                .lock()
                                .unwrap()
                                .insert(handle_id, conn.chan.sender());
                            conn.run();
                            info!("[{}] disconnected peer #{}", addr, handle_id);

                            senders.lock().unwrap().remove(&handle_id);
                            handles.lock().unwrap().remove(&handle_id);
                        })
                    };

                    handles.lock().unwrap().insert(handle_id, handle);
                }
                trace!("closing {} RPC connections", senders.lock().unwrap().len());
                for sender in senders.lock().unwrap().values() {
                    let _ = sender.send(Message::Done);
                }

                trace!(
                    "waiting for {} RPC handling threads",
                    handles.lock().unwrap().len()
                );

                let handle_ids: Vec<i32> =
                    handles.lock().unwrap().keys().map(|i| i.clone()).collect();
                for id in handle_ids {
                    let h = handles.lock().unwrap().remove(&id);
                    match h {
                        Some(h) => {
                            if let Err(e) = h.join() {
                                warn!("failed to join thread: {:?}", e);
                            }
                        }
                        None => {}
                    }
                }

                trace!("RPC connections are closed");
            })),
        }
    }

    fn get_scripthashes_effected_by_tx(
        &self,
        txid: &Sha256dHash,
        blockhash: Option<&Sha256dHash>,
    ) -> Result<Vec<FullHash>> {
        let txn = self.query.load_txn(txid, blockhash, None)?;
        let mut scripthashes = get_output_scripthash(&txn, None);

        for txin in txn.input {
            if txin.previous_output.is_null() {
                continue;
            }
            let id: &Sha256dHash = &txin.previous_output.txid;
            let n = txin.previous_output.vout as usize;

            let txn = self.query.load_txn(&id, None, None)?;
            scripthashes.extend(get_output_scripthash(&txn, Some(n)));
        }
        Ok(scripthashes)
    }

    pub fn notify_scripthash_subscriptions(
        &self,
        headers_changed: &Vec<HeaderEntry>,
        txs_changed: HashSet<Sha256dHash>,
    ) {
        let mut txn_done: HashSet<Sha256dHash> = HashSet::new();
        let mut scripthashes: HashSet<FullHash> = HashSet::new();

        let mut insert_for_tx = |txid, blockhash| {
            if !txn_done.insert(txid) {
                return;
            }
            if let Ok(hashes) = self.get_scripthashes_effected_by_tx(&txid, blockhash) {
                for h in hashes {
                    scripthashes.insert(h);
                }
            } else {
                trace!("failed to get effected scripthashes for tx {}", txid);
            }
        };

        for header in headers_changed {
            let blockhash = header.hash();
            let txids = match self.query.getblocktxids(&blockhash) {
                Ok(txids) => txids,
                Err(e) => {
                    warn!("Failed to get blocktxids for {}: {}", blockhash, e);
                    continue;
                }
            };
            for txid in txids {
                insert_for_tx(txid, Some(blockhash));
            }
        }
        for txid in txs_changed {
            insert_for_tx(txid, None);
        }

        for s in scripthashes.drain() {
            if let Err(e) = self.notification.send(Notification::ScriptHashChange(s)) {
                trace!("Scripthash change notification failed: {}", e);
            }
        }
    }

    pub fn notify_subscriptions_chaintip(&self, header: HeaderEntry) {
        if let Err(e) = self.notification.send(Notification::ChainTipChange(header)) {
            trace!("Failed to notify about chaintip change {}", e);
        }
    }

    pub fn disconnect_clients(&self) {
        trace!("disconncting clients");
        self.notification.send(Notification::Exit).unwrap();
    }
}

impl Drop for RPC {
    fn drop(&mut self) {
        trace!("stop accepting new RPCs");
        self.notification.send(Notification::Exit).unwrap();
        if let Some(handle) = self.server.take() {
            handle.join().unwrap();
        }
        trace!("RPC server is stopped");
    }
}
