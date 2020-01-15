use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::{deserialize, serialize};
use bitcoin_hashes::hex::{FromHex, ToHex};
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use bitcoin_hashes::Hash;
use error_chain::ChainedError;
use hex;
use serde_json::{from_str, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::errors::*;
use crate::index::compute_script_hash;
use crate::metrics::{Gauge, HistogramOpts, HistogramVec, MetricOpts, Metrics};
use crate::query::{Query, Status};
use crate::util::FullHash;
use crate::util::{spawn_thread, Channel, HeaderEntry, SyncChannel};

const ELECTRSCASH_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "1.4";
const PROTOCOL_HASH_FUNCTION: &str = "sha256";

// TODO: Sha256dHash should be a generic hash-container (since script hash is single SHA256)
fn hash_from_value(val: Option<&Value>) -> Result<Sha256dHash> {
    let script_hash = val.chain_err(|| "missing hash")?;
    let script_hash = script_hash.as_str().chain_err(|| "non-string hash")?;
    let script_hash = Sha256dHash::from_hex(script_hash).chain_err(|| "non-hex hash")?;
    Ok(script_hash)
}

fn usize_from_value(val: Option<&Value>, name: &str) -> Result<usize> {
    let val = val.chain_err(|| format!("missing {}", name))?;
    let val = val.as_u64().chain_err(|| format!("non-integer {}", name))?;
    Ok(val as usize)
}

fn usize_from_value_or(val: Option<&Value>, name: &str, default: usize) -> Result<usize> {
    if val.is_none() {
        return Ok(default);
    }
    usize_from_value(val, name)
}

fn bool_from_value(val: Option<&Value>, name: &str) -> Result<bool> {
    let val = val.chain_err(|| format!("missing {}", name))?;
    let val = val.as_bool().chain_err(|| format!("not a bool {}", name))?;
    Ok(val)
}

fn bool_from_value_or(val: Option<&Value>, name: &str, default: bool) -> Result<bool> {
    if val.is_none() {
        return Ok(default);
    }
    bool_from_value(val, name)
}

fn unspent_from_status(status: &Status) -> Value {
    json!(Value::Array(
        status
            .unspent()
            .into_iter()
            .map(|out| json!({
                "height": out.height,
                "tx_pos": out.output_index,
                "tx_hash": out.txn_id.to_hex(),
                "value": out.value,
            }))
            .collect()
    ))
}

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
    last_header_entry: Option<HeaderEntry>,
    status_hashes: HashMap<Sha256dHash, Value>, // ScriptHash -> StatusHash
    stream: TcpStream,
    addr: SocketAddr,
    chan: SyncChannel<Message>,
    stats: Arc<Stats>,
}

impl Connection {
    pub fn new(
        query: Arc<Query>,
        stream: TcpStream,
        addr: SocketAddr,
        stats: Arc<Stats>,
    ) -> Connection {
        Connection {
            query,
            last_header_entry: None, // disable header subscription for now
            status_hashes: HashMap::new(),
            stream,
            addr,
            chan: SyncChannel::new(10),
            stats,
        }
    }

    fn blockchain_headers_subscribe(&mut self) -> Result<Value> {
        let entry = self.query.get_best_header()?;
        let hex_header = hex::encode(serialize(entry.header()));
        let result = json!({"hex": hex_header, "height": entry.height()});
        self.last_header_entry = Some(entry);
        Ok(result)
    }

    fn server_version(&self) -> Result<Value> {
        Ok(json!([
            format!("ElectrsCash {}", ELECTRSCASH_VERSION),
            PROTOCOL_VERSION
        ]))
    }

    fn server_banner(&self) -> Result<Value> {
        Ok(json!(self.query.get_banner()?))
    }

    fn server_donation_address(&self) -> Result<Value> {
        Ok(Value::Null)
    }

    fn server_peers_subscribe(&self) -> Result<Value> {
        Ok(json!([]))
    }

    fn server_features(&self) -> Result<Value> {
        let genesis_header = self.query.get_headers(&[0])[0].clone();
        Ok(json!({
            "genesis_hash" : genesis_header.hash().to_hex(),
            "hash_function": PROTOCOL_HASH_FUNCTION,
            "protocol_max": PROTOCOL_VERSION,
            "protocol_min": PROTOCOL_VERSION,
            "server_version": format!("ElectrsCash {}", ELECTRSCASH_VERSION)
        }))
    }

    fn mempool_get_fee_histogram(&self) -> Result<Value> {
        Ok(json!(self.query.get_fee_histogram()))
    }

    fn blockchain_block_header(&self, params: &[Value]) -> Result<Value> {
        let height = usize_from_value(params.get(0), "height")?;
        let cp_height = usize_from_value_or(params.get(1), "cp_height", 0)?;

        let raw_header_hex: String = self
            .query
            .get_headers(&[height])
            .into_iter()
            .map(|entry| hex::encode(&serialize(entry.header())))
            .collect();

        if cp_height == 0 {
            return Ok(json!(raw_header_hex));
        }
        let (branch, root) = self.query.get_header_merkle_proof(height, cp_height)?;

        let branch_vec: Vec<String> = branch.into_iter().map(|b| b.to_hex()).collect();

        Ok(json!({
            "header": raw_header_hex,
            "root": root.to_hex(),
            "branch": branch_vec
        }))
    }

    fn blockchain_block_headers(&self, params: &[Value]) -> Result<Value> {
        let start_height = usize_from_value(params.get(0), "start_height")?;
        let count = usize_from_value(params.get(1), "count")?;
        let cp_height = usize_from_value_or(params.get(2), "cp_height", 0)?;
        let heights: Vec<usize> = (start_height..(start_height + count)).collect();
        let headers: Vec<String> = self
            .query
            .get_headers(&heights)
            .into_iter()
            .map(|entry| hex::encode(&serialize(entry.header())))
            .collect();

        if count == 0 || cp_height == 0 {
            return Ok(json!({
                "count": headers.len(),
                "hex": headers.join(""),
                "max": 2016,
            }));
        }

        let (branch, root) = self
            .query
            .get_header_merkle_proof(start_height + (count - 1), cp_height)?;

        let branch_vec: Vec<String> = branch.into_iter().map(|b| b.to_hex()).collect();

        Ok(json!({
            "count": headers.len(),
            "hex": headers.join(""),
            "max": 2016,
            "root": root.to_hex(),
            "branch" : branch_vec
        }))
    }

    fn blockchain_estimatefee(&self, params: &[Value]) -> Result<Value> {
        let blocks_count = usize_from_value(params.get(0), "blocks_count")?;
        let fee_rate = self.query.estimate_fee(blocks_count); // in BTC/kB
        Ok(json!(fee_rate))
    }

    fn blockchain_relayfee(&self) -> Result<Value> {
        Ok(json!(0.0)) // allow sending transactions with any fee.
    }

    fn blockchain_scripthash_subscribe(&mut self, params: &[Value]) -> Result<Value> {
        let script_hash = hash_from_value(params.get(0)).chain_err(|| "bad script_hash")?;
        let status = self.query.status(&script_hash[..])?;
        let result = status.hash().map_or(Value::Null, |h| json!(hex::encode(h)));
        self.status_hashes.insert(script_hash, result.clone());
        self.stats
            .subscriptions
            .set(self.status_hashes.len() as i64);
        Ok(result)
    }

    fn blockchain_scripthash_get_balance(&self, params: &[Value]) -> Result<Value> {
        let script_hash = hash_from_value(params.get(0)).chain_err(|| "bad script_hash")?;
        let status = self.query.status(&script_hash[..])?;
        Ok(
            json!({ "confirmed": status.confirmed_balance(), "unconfirmed": status.mempool_balance() }),
        )
    }

    fn blockchain_scripthash_get_history(&self, params: &[Value]) -> Result<Value> {
        let script_hash = hash_from_value(params.get(0)).chain_err(|| "bad script_hash")?;
        let status = self.query.status(&script_hash[..])?;
        Ok(json!(Value::Array(
            status
                .history()
                .into_iter()
                .map(|item| json!({"height": item.0, "tx_hash": item.1.to_hex()}))
                .collect()
        )))
    }

    fn blockchain_scripthash_listunspent(&self, params: &[Value]) -> Result<Value> {
        let script_hash = hash_from_value(params.get(0)).chain_err(|| "bad script_hash")?;
        Ok(unspent_from_status(&self.query.status(&script_hash[..])?))
    }

    fn blockchain_transaction_broadcast(&self, params: &[Value]) -> Result<Value> {
        let tx = params.get(0).chain_err(|| "missing tx")?;
        let tx = tx.as_str().chain_err(|| "non-string tx")?;
        let tx = hex::decode(&tx).chain_err(|| "non-hex tx")?;
        let tx: Transaction = deserialize(&tx).chain_err(|| "failed to parse tx")?;
        let txid = self.query.broadcast(&tx)?;
        Ok(json!(txid.to_hex()))
    }

    fn blockchain_transaction_get(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = hash_from_value(params.get(0)).chain_err(|| "bad tx_hash")?;
        let verbose = match params.get(1) {
            Some(value) => value.as_bool().chain_err(|| "non-bool verbose value")?,
            None => false,
        };
        Ok(self.query.get_transaction(&tx_hash, verbose)?)
    }

    fn blockchain_transaction_get_merkle(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = hash_from_value(params.get(0)).chain_err(|| "bad tx_hash")?;
        let height = usize_from_value(params.get(1), "height")?;
        let (merkle, pos) = self
            .query
            .get_merkle_proof(&tx_hash, height)
            .chain_err(|| "cannot create merkle proof")?;
        let merkle: Vec<String> = merkle.into_iter().map(|txid| txid.to_hex()).collect();
        Ok(json!({
                "block_height": height,
                "merkle": merkle,
                "pos": pos}))
    }

    fn blockchain_transaction_id_from_pos(&self, params: &[Value]) -> Result<Value> {
        let height = usize_from_value(params.get(0), "height")?;
        let tx_pos = usize_from_value(params.get(1), "tx_pos")?;
        let want_merkle = bool_from_value_or(params.get(2), "merkle", false)?;

        let (txid, merkle) = self.query.get_id_from_pos(height, tx_pos, want_merkle)?;

        if !want_merkle {
            return Ok(json!(txid.to_hex()));
        }

        let merkle_vec: Vec<String> = merkle.into_iter().map(|entry| entry.to_hex()).collect();

        Ok(json!({
            "tx_hash" : txid.to_hex(),
            "merkle" : merkle_vec}))
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
        let result = match method {
            "blockchain.block.header" => self.blockchain_block_header(&params),
            "blockchain.block.headers" => self.blockchain_block_headers(&params),
            "blockchain.estimatefee" => self.blockchain_estimatefee(&params),
            "blockchain.headers.subscribe" => self.blockchain_headers_subscribe(),
            "blockchain.relayfee" => self.blockchain_relayfee(),
            "blockchain.scripthash.get_balance" => self.blockchain_scripthash_get_balance(&params),
            "blockchain.scripthash.get_history" => self.blockchain_scripthash_get_history(&params),
            "blockchain.scripthash.listunspent" => self.blockchain_scripthash_listunspent(&params),
            "blockchain.scripthash.subscribe" => self.blockchain_scripthash_subscribe(&params),
            "blockchain.transaction.broadcast" => self.blockchain_transaction_broadcast(&params),
            "blockchain.transaction.get" => self.blockchain_transaction_get(&params),
            "blockchain.transaction.get_merkle" => self.blockchain_transaction_get_merkle(&params),
            "blockchain.transaction.id_from_pos" => {
                self.blockchain_transaction_id_from_pos(&params)
            }
            "mempool.get_fee_histogram" => self.mempool_get_fee_histogram(),
            "server.banner" => self.server_banner(),
            "server.donation_address" => self.server_donation_address(),
            "server.peers.subscribe" => self.server_peers_subscribe(),
            "server.ping" => Ok(Value::Null),
            "server.version" => self.server_version(),
            "server.features" => self.server_features(),
            "cashaccount.query.name" => self.cashaccount_query_name(&params),
            &_ => bail!("unknown method {} {:?}", method, params),
        };
        timer.observe_duration();
        // TODO: return application errors should be sent to the client
        Ok(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => {
                warn!(
                    "rpc #{} {} {:?} failed: {}",
                    id,
                    method,
                    params,
                    e.display_chain()
                );
                json!({"jsonrpc": "2.0", "id": id, "error": format!("{}", e)})
            }
        })
    }

    fn on_scripthash_change(&mut self, scripthash: FullHash) -> Result<()> {
        let scripthash = Sha256dHash::from_slice(&scripthash[..]).expect("invalid scripthash");

        let old_statushash;
        match self.status_hashes.get(&scripthash) {
            Some(statushash) => {
                old_statushash = statushash;
            }
            None => {
                return Ok(());
            }
        };

        let timer = self
            .stats
            .latency
            .with_label_values(&["statushash_update"])
            .start_timer();

        let status = self.query.status(&scripthash[..])?;
        let new_statushash = status.hash().map_or(Value::Null, |h| json!(hex::encode(h)));
        if new_statushash == *old_statushash {
            return Ok(());
        }
        timer.observe_duration();
        self.send_values(&vec![json!({
                    "jsonrpc": "2.0",
                    "method": "blockchain.scripthash.subscribe",
                    "params": [scripthash.to_hex(), new_statushash]})])?;
        self.status_hashes.insert(scripthash, new_statushash);
        Ok(())
    }

    fn on_chaintip_change(&mut self, chaintip: HeaderEntry) -> Result<()> {
        let timer = self
            .stats
            .latency
            .with_label_values(&["chaintip_update"])
            .start_timer();

        if let Some(ref mut last_entry) = self.last_header_entry {
            if *last_entry == chaintip {
                return Ok(());
            }

            *last_entry = chaintip;
            let hex_header = hex::encode(serialize(last_entry.header()));
            let header = json!({"hex": hex_header, "height": last_entry.height()});
            self.send_values(&vec![
                (json!({
                "jsonrpc": "2.0",
                "method": "blockchain.headers.subscribe",
                "params": [header]})),
            ])?;
        }
        timer.observe_duration();
        Ok(())
    }

    fn send_values(&mut self, values: &[Value]) -> Result<()> {
        for value in values {
            let line = value.to_string() + "\n";
            self.stream
                .write_all(line.as_bytes())
                .chain_err(|| format!("failed to send {}", value))?;
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
                Message::ScriptHashChange(hash) => self.on_scripthash_change(hash)?,
                Message::ChainTipChange(tip) => self.on_chaintip_change(tip)?,
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
            error!(
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
    subscriptions: Gauge,
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
                                warn!("failed to send ScriptHashChange to peer {}: {}", i, e);
                            }
                        }
                    }
                    Notification::ChainTipChange(hash) => {
                        for (i, sender) in senders.iter() {
                            if let Err(e) = sender.try_send(Message::ChainTipChange(hash.clone())) {
                                warn!("failed to send ChainTipChange to peer {}: {}", i, e);
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
                addr, PROTOCOL_VERSION
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

    pub fn start(addr: SocketAddr, query: Arc<Query>, metrics: &Metrics) -> RPC {
        let stats = Arc::new(Stats {
            latency: metrics.histogram_vec(
                HistogramOpts::new("electrscash_electrum_rpc", "Electrum RPC latency (seconds)"),
                &["method"],
            ),
            subscriptions: metrics.gauge(MetricOpts::new(
                "electrscash_electrum_subscriptions",
                "# of Electrum subscriptions",
            )),
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

                        spawn_thread("peer", move || {
                            info!("[{}] connected peer #{}", addr, handle_id);
                            let conn = Connection::new(query, stream, addr, stats);
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
        blockhash: Option<Sha256dHash>,
    ) -> Result<Vec<FullHash>> {
        let txn = self.query.load_txn_with_blockhashlookup(txid, blockhash)?;
        let mut scripthashes = get_output_scripthash(&txn, None);

        for txin in txn.input {
            if txin.previous_output.is_null() {
                continue;
            }
            let id: &Sha256dHash = &txin.previous_output.txid;
            let n = txin.previous_output.vout as usize;

            let txn = self.query.load_txn_with_blockhashlookup(&id, None)?;
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
                insert_for_tx(txid, Some(*blockhash));
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
