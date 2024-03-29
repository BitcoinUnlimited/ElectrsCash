use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::hash_types::{BlockHash, Txid};
use error_chain::ChainedError;
use serde_json::{from_str, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::def::PROTOCOL_VERSION_MAX;
use crate::doslimit::{ConnectionLimits, GlobalLimits};
use crate::errors::*;
use crate::metrics::Metrics;
use crate::query::Query;
use crate::rpc::blockchain::BlockchainRpc;
use crate::rpc::parseutil::usize_from_value;
use crate::rpc::rpcstats::RpcStats;
use crate::rpc::server::{
    server_add_peer, server_banner, server_donation_address, server_features,
    server_peers_subscribe, server_version,
};
use crate::scripthash::{compute_script_hash, FullHash};
use crate::timeout::TimeoutTrigger;
use crate::util::{spawn_thread, Channel, HeaderEntry};

pub mod blockchain;
pub mod parseutil;
pub mod rpcstats;
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
    sender: SyncSender<Message>,
    stats: Arc<RpcStats>,
    doslimits: ConnectionLimits,
    blockchainrpc: BlockchainRpc,
}

impl Connection {
    pub fn new(
        query: Arc<Query>,
        stream: TcpStream,
        addr: SocketAddr,
        stats: Arc<RpcStats>,
        relayfee: f64,
        doslimits: ConnectionLimits,
        sender: SyncSender<Message>,
    ) -> Connection {
        Connection {
            query: query.clone(),
            stream,
            addr,
            sender,
            stats: stats.clone(),
            doslimits,
            blockchainrpc: BlockchainRpc::new(query, stats, relayfee, doslimits),
        }
    }

    fn mempool_get_fee_histogram(&self) -> Value {
        json!(self.query.get_fee_histogram())
    }

    fn cashaccount_query_name(&self, params: &[Value]) -> Result<Value> {
        let name = params.get(0).chain_err(|| "missing name")?;
        let name = name.as_str().chain_err(|| "bad accountname")?;
        let height = usize_from_value(params.get(1), "height")?;

        self.query.get_cashaccount_txs(name, height as u32)
    }

    fn handle_command(&mut self, method: &str, params: &[Value], id: &Value) -> Value {
        let timer = self
            .stats
            .latency
            .with_label_values(&[method])
            .start_timer();
        let timeout = TimeoutTrigger::new(Duration::from_secs(self.doslimits.rpc_timeout as u64));
        let result = match method {
            "blockchain.address.get_balance" => {
                self.blockchainrpc.address_get_balance(params, &timeout)
            }
            "blockchain.address.get_first_use" => self.blockchainrpc.address_get_first_use(params),
            "blockchain.address.get_history" => {
                self.blockchainrpc.address_get_history(params, &timeout)
            }
            "blockchain.address.get_mempool" => {
                self.blockchainrpc.address_get_mempool(params, &timeout)
            }
            "blockchain.address.get_scripthash" => {
                self.blockchainrpc.address_get_scripthash(params)
            }
            "blockchain.address.subscribe" => {
                self.blockchainrpc.address_subscribe(params, &timeout)
            }
            "blockchain.address.listunspent" => {
                self.blockchainrpc.address_listunspent(params, &timeout)
            }
            "blockchain.address.unsubscribe" => self.blockchainrpc.address_unsubscribe(params),
            "blockchain.block.header" => self.blockchainrpc.block_header(params),
            "blockchain.block.headers" => self.blockchainrpc.block_headers(params),
            "blockchain.estimatefee" => self.blockchainrpc.estimatefee(params),
            "blockchain.headers.subscribe" => self.blockchainrpc.headers_subscribe(),
            "blockchain.relayfee" => self.blockchainrpc.relayfee(),
            "blockchain.scripthash.get_balance" => {
                self.blockchainrpc.scripthash_get_balance(params, &timeout)
            }
            "blockchain.scripthash.get_first_use" => {
                self.blockchainrpc.scripthash_get_first_use(params)
            }
            "blockchain.scripthash.get_history" => {
                self.blockchainrpc.scripthash_get_history(params, &timeout)
            }
            "blockchain.scripthash.get_mempool" => {
                self.blockchainrpc.scripthash_get_mempool(params, &timeout)
            }
            "blockchain.scripthash.listunspent" => {
                self.blockchainrpc.scripthash_listunspent(params, &timeout)
            }
            "blockchain.scripthash.subscribe" => {
                self.blockchainrpc.scripthash_subscribe(params, &timeout)
            }
            "blockchain.scripthash.unsubscribe" => {
                self.blockchainrpc.scripthash_unsubscribe(params)
            }
            "blockchain.transaction.broadcast" => self.blockchainrpc.transaction_broadcast(params),
            "blockchain.transaction.get" => self.blockchainrpc.transaction_get(params),
            "blockchain.transaction.get_confirmed_blockhash" => self
                .blockchainrpc
                .transaction_get_confirmed_blockhash(params),
            "blockchain.transaction.get_merkle" => {
                self.blockchainrpc.transaction_get_merkle(params)
            }
            "blockchain.transaction.id_from_pos" => {
                self.blockchainrpc.transaction_id_from_pos(params)
            }
            "blockchain.utxo.get" => self.blockchainrpc.utxo_get(params, &timeout),
            "mempool.get_fee_histogram" => Ok(self.mempool_get_fee_histogram()),
            "server.add_peer" => server_add_peer(),
            "server.banner" => server_banner(&self.query),
            "server.donation_address" => server_donation_address(),
            "server.features" => server_features(&self.query),
            "server.peers.subscribe" => server_peers_subscribe(),
            "server.ping" => Ok(Value::Null),
            "server.version" => server_version(params),
            "cashaccount.query.name" => self.cashaccount_query_name(params),
            &_ => Err(ErrorKind::RpcError(
                RpcErrorCode::MethodNotFound,
                format!("unknown method {}", method),
            )
            .into()),
        };
        timer.observe_duration();
        // TODO: return application errors should be sent to the client
        if let Err(e) = result {
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
        }
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

    fn handle_replies(&mut self, receiver: Receiver<Message>) -> Result<()> {
        let empty_params = json!([]);
        loop {
            let msg = receiver.recv().chain_err(|| "channel closed")?;
            match msg {
                Message::Request(line) => {
                    trace!("RPC {:?}", line);
                    let cmd: Value = from_str(&line).chain_err(|| "invalid JSON format")?;
                    let reply = match (
                        cmd.get("method"),
                        cmd.get("params").unwrap_or(&empty_params),
                        cmd.get("id"),
                    ) {
                        (Some(&Value::String(ref method)), &Value::Array(ref params), Some(id)) => {
                            self.handle_command(method, params, id)
                        }
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

    fn parse_requests(mut reader: BufReader<TcpStream>, tx: SyncSender<Message>) -> Result<()> {
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

    pub fn run(mut self, receiver: Receiver<Message>) {
        let reader = BufReader::new(self.stream.try_clone().expect("failed to clone TcpStream"));
        let sender = self.sender.clone();
        let child = spawn_thread("reader", || Connection::parse_requests(reader, sender));
        if let Err(e) = self.handle_replies(receiver) {
            error!(
                "[{}] connection handling failed: {}",
                self.addr,
                e.display_chain().to_string()
            );
        }
        self.stats
            .subscriptions
            .sub(self.blockchainrpc.get_num_subscriptions());
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

pub struct Rpc {
    notification: Sender<Notification>,
    server: Option<thread::JoinHandle<()>>, // so we can join the server while dropping this ojbect
    query: Arc<Query>,
}

impl Rpc {
    fn start_notifier(
        notification: Channel<Notification>,
        senders: Arc<Mutex<Vec<SyncSender<Message>>>>,
        acceptor: Sender<Option<(TcpStream, SocketAddr)>>,
    ) {
        spawn_thread("notification", move || {
            for msg in notification.receiver().iter() {
                let mut senders = senders.lock().unwrap();
                match msg {
                    Notification::ScriptHashChange(hash) => senders.retain(|sender| {
                        if let Err(TrySendError::Disconnected(_)) =
                            sender.try_send(Message::ScriptHashChange(hash))
                        {
                            debug!("peer disconnected");
                            false
                        } else {
                            true
                        }
                    }),
                    Notification::ChainTipChange(hash) => senders.retain(|sender| {
                        if let Err(TrySendError::Disconnected(_)) =
                            sender.try_send(Message::ChainTipChange(hash.clone()))
                        {
                            debug!("peer disconnected");
                            false
                        } else {
                            true
                        }
                    }),
                    // mark acceptor as done
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
                match acceptor.send(Some((stream, addr))) {
                    Ok(_) => {}
                    Err(e) => trace!("Failed to send to client {:?}", e),
                }
            }
        });
        chan
    }

    pub fn start(
        addr: SocketAddr,
        query: Arc<Query>,
        metrics: Arc<Metrics>,
        relayfee: f64,
        connection_limits: ConnectionLimits,
        global_limits: Arc<GlobalLimits>,
        rpc_buffer_size: usize,
    ) -> Rpc {
        let stats = Arc::new(RpcStats {
            latency: metrics.histogram_vec(
                prometheus::HistogramOpts::new("electrscash_rpc_latency", "RPC latency (seconds)"),
                &["method"],
            ),
            subscriptions: metrics.gauge_int(prometheus::Opts::new(
                "electrscash_scripthash_subscriptions",
                "# of scripthash subscriptions for node",
            )),
        });

        stats.subscriptions.set(0);
        let notification = Channel::unbounded();
        Rpc {
            notification: notification.sender(),
            query: query.clone(),
            server: Some(spawn_thread("rpc", move || {
                let senders = Arc::new(Mutex::new(Vec::<SyncSender<Message>>::new()));

                let acceptor = Rpc::start_acceptor(addr);
                Rpc::start_notifier(notification, senders.clone(), acceptor.sender());

                let mut threads = HashMap::new();
                let (garbage_sender, garbage_receiver) = crossbeam_channel::unbounded();

                while let Some((stream, addr)) = acceptor.receiver().recv().unwrap() {
                    let global_limits = global_limits.clone();

                    let mut connections = match global_limits.inc_connection(&addr.ip()) {
                        Err(e) => {
                            trace!("[{}] dropping peer - {}", addr, e);
                            let _ = stream.shutdown(Shutdown::Both);
                            continue;
                        }
                        Ok(n) => n,
                    };
                    // explicitely scope the shadowed variables for the new thread
                    let query = Arc::clone(&query);
                    let stats = Arc::clone(&stats);
                    let garbage_sender = garbage_sender.clone();
                    let (sender, receiver) = mpsc::sync_channel(rpc_buffer_size);

                    senders.lock().unwrap().push(sender.clone());

                    let spawned = spawn_thread("peer", move || {
                        info!(
                            "[{}] connected peer ({:?} out of {:?} connection slots used)",
                            addr,
                            connections,
                            global_limits.connection_limits(),
                        );
                        let conn = Connection::new(
                            query,
                            stream,
                            addr,
                            stats,
                            relayfee,
                            connection_limits,
                            sender,
                        );
                        conn.run(receiver);
                        match global_limits.dec_connection(&addr.ip()) {
                            Ok(n) => connections = n,
                            Err(e) => error!("{}", e),
                        };
                        info!(
                            "[{}] disconnected peer ({:?} out of {:?} connection slots used)",
                            addr,
                            connections,
                            global_limits.connection_limits(),
                        );
                        let _ = garbage_sender.send(std::thread::current().id());
                    });

                    trace!("[{}] spawned {:?}", addr, spawned.thread().id());
                    threads.insert(spawned.thread().id(), spawned);
                    while let Ok(id) = garbage_receiver.try_recv() {
                        if let Some(thread) = threads.remove(&id) {
                            trace!("[{}] joining {:?}", addr, id);
                            if let Err(error) = thread.join() {
                                error!("failed to join {:?}: {:?}", id, error);
                            }
                        }
                    }
                }
                info!("closing {} RPC connections", senders.lock().unwrap().len());
                for sender in senders.lock().unwrap().iter() {
                    let _ = sender.send(Message::Done);
                }

                info!("waiting for {} RPC handling threads", threads.len());

                for (id, thread) in threads {
                    trace!("joining {:?}", id);
                    if let Err(error) = thread.join() {
                        error!("failed to join {:?}: {:?}", id, error);
                    }
                }
                info!("RPC connections are closed");
            })),
        }
    }

    fn get_scripthashes_effected_by_tx(
        &self,
        txid: &Txid,
        blockhash: Option<&BlockHash>,
    ) -> Result<Vec<FullHash>> {
        let txn = self.query.tx().get(txid, blockhash, None)?;
        let mut scripthashes = get_output_scripthash(&txn, None);

        for txin in txn.input {
            if txin.previous_output.is_null() {
                continue;
            }
            let id: &Txid = &txin.previous_output.txid;
            let n = txin.previous_output.vout as usize;

            let txn = self.query.tx().get(id, None, None)?;
            scripthashes.extend(get_output_scripthash(&txn, Some(n)));
        }
        Ok(scripthashes)
    }

    pub fn notify_scripthash_subscriptions(
        &self,
        headers_changed: &[HeaderEntry],
        txs_changed: HashSet<Txid>,
    ) {
        let mut txn_done: HashSet<Txid> = HashSet::new();
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
            let txids = match self.query.getblocktxids(blockhash) {
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

impl Drop for Rpc {
    fn drop(&mut self) {
        trace!("stop accepting new RPCs");
        self.notification.send(Notification::Exit).unwrap();
        if let Some(handle) = self.server.take() {
            handle.join().unwrap();
        }
        trace!("RPC server is stopped");
    }
}
