use crate::errors::*;
use crate::metrics::{Gauge, HistogramOpts, HistogramVec, MetricOpts, Metrics};
use crate::query::Query;
use crate::rpc::parseutil::{
    bool_from_value_or, rpc_arg_error, scripthash_from_value, sha256d_from_value, str_from_value,
    usize_from_value, usize_from_value_or,
};
use crate::rpc::scripthash::{get_balance, get_first_use, get_history, listunspent};
use crate::scripthash::addr_to_scripthash;
use crate::scripthash::{FullHash, ToLEHex};
use crate::timeout::TimeoutTrigger;
use crate::util::HeaderEntry;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::{deserialize, serialize};
use bitcoin_hashes::hex::ToHex;
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use hex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

struct BlockchainRPCStats {
    subscriptions: Gauge,
    latency: HistogramVec,
}

pub struct BlockchainRPC {
    query: Arc<Query>,
    stats: BlockchainRPCStats,
    status_hashes: HashMap<FullHash, Value>, // ScriptHash -> StatusHash
    last_header_entry: Option<HeaderEntry>,
    relayfee: f64,
    rpc_timeout: u16,
}

impl BlockchainRPC {
    pub fn new(
        query: Arc<Query>,
        metrics: Arc<Metrics>,
        relayfee: f64,
        rpc_timeout: u16,
    ) -> BlockchainRPC {
        let stats = BlockchainRPCStats {
            subscriptions: metrics.gauge(MetricOpts::new(
                "electrscash_scripthash_subscriptions",
                "# of scripthash subscriptions",
            )),
            latency: metrics.histogram_vec(
                HistogramOpts::new(
                    "electrscash_rpc_blockchain",
                    "Electrum blockchain RPC latency (seconds)",
                ),
                &["method"],
            ),
        };
        BlockchainRPC {
            query,
            stats,
            status_hashes: HashMap::new(),
            last_header_entry: None, // disable header subscription for now
            relayfee,
            rpc_timeout,
        }
    }
    pub fn address_get_balance(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        get_balance(&*self.query, &scripthash, timeout)
    }
    pub fn address_get_first_use(&self, params: &[Value]) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        get_first_use(&*self.query, &scripthash)
    }
    pub fn address_get_history(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        get_history(&self.query, &scripthash, timeout)
    }
    pub fn address_listunspent(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        listunspent(&*self.query, &scripthash, timeout)
    }

    pub fn block_header(&self, params: &[Value]) -> Result<Value> {
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

    pub fn block_headers(&self, params: &[Value]) -> Result<Value> {
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

    pub fn estimatefee(&self, params: &[Value]) -> Result<Value> {
        let blocks_count = usize_from_value(params.get(0), "blocks_count")?;
        let fee_rate = self.query.estimate_fee(blocks_count); // in BCH/kB
        Ok(json!(fee_rate.max(self.relayfee)))
    }

    pub fn headers_subscribe(&mut self) -> Result<Value> {
        let entry = self.query.get_best_header()?;
        let hex_header = hex::encode(serialize(entry.header()));
        let result = json!({"hex": hex_header, "height": entry.height()});
        self.last_header_entry = Some(entry);
        Ok(result)
    }

    pub fn relayfee(&self) -> Result<Value> {
        Ok(json!(self.relayfee)) // in BTC/kB
    }

    pub fn scripthash_get_balance(
        &self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        get_balance(&*self.query, &scripthash, timeout)
    }

    pub fn scripthash_get_first_use(&self, params: &[Value]) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        get_first_use(&*self.query, &scripthash)
    }

    pub fn scripthash_get_history(
        &self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        get_history(&self.query, &scripthash, timeout)
    }

    pub fn scripthash_listunspent(
        &self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        listunspent(&*self.query, &scripthash, timeout)
    }

    pub fn scripthash_subscribe(
        &mut self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let script_hash = scripthash_from_value(params.get(0))?;
        let status = self.query.status(&script_hash, timeout)?;
        let result = status.hash().map_or(Value::Null, |h| json!(hex::encode(h)));
        self.status_hashes.insert(script_hash, result.clone());
        self.stats
            .subscriptions
            .set(self.status_hashes.len() as i64);
        Ok(result)
    }

    pub fn scripthash_unsubscribe(&mut self, params: &[Value]) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        let removed = self.status_hashes.remove(&scripthash).is_some();
        Ok(json!(removed))
    }

    pub fn transaction_broadcast(&self, params: &[Value]) -> Result<Value> {
        let tx = params.get(0).chain_err(|| rpc_arg_error("missing tx"))?;
        let tx = tx.as_str().chain_err(|| rpc_arg_error("non-string tx"))?;
        let tx = hex::decode(&tx).chain_err(|| rpc_arg_error("non-hex tx"))?;
        let tx: Transaction = deserialize(&tx).chain_err(|| rpc_arg_error("failed to parse tx"))?;
        let txid = self
            .query
            .broadcast(&tx)
            .chain_err(|| rpc_arg_error("rejected by network"))?;
        Ok(json!(txid.to_hex()))
    }

    pub fn transaction_get(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = sha256d_from_value(params.get(0))?;
        let verbose = match params.get(1) {
            Some(value) => value.as_bool().chain_err(|| "non-bool verbose value")?,
            None => false,
        };
        if !verbose {
            let tx = self.query.load_txn(&tx_hash, None, None)?;
            Ok(json!(hex::encode(serialize(&tx))))
        } else {
            let header = self.query.lookup_blockheader(&tx_hash, None)?;
            let blocktime = match header {
                Some(ref header) => header.header().time,
                None => 0,
            };
            let height = match header {
                Some(ref header) => header.height(),
                None => 0,
            };
            let confirmations = match header {
                Some(ref header) => {
                    if let Some(best) = self.query.best_header() {
                        best.height() - header.height()
                    } else {
                        0
                    }
                }
                None => 0,
            };
            let blockhash = header.and_then(|h| Some(h.hash().clone()));
            let tx = self.query.load_txn(&tx_hash, blockhash.as_ref(), None)?;

            let tx_serialized = serialize(&tx);
            Ok(json!({
                "blockhash": blockhash.unwrap_or(Sha256dHash::default()).to_hex(),
                "blocktime": blocktime,
                "height": height,
                "confirmations": confirmations,
                "hash": tx.txid().to_hex(),
                "txid": tx.txid().to_hex(),
                "size": tx_serialized.len(),
                "hex": hex::encode(tx_serialized),
                "locktime": tx.lock_time,
                "time": blocktime,
                "vin": tx.input.iter().map(|txin| json!({
                    "sequence": txin.sequence,
                    "txid": txin.previous_output.txid.to_hex(),
                    "vout": txin.previous_output.vout,
                    "scriptSig": txin.script_sig.to_hex(),
                })).collect::<Vec<Value>>(),
                "vout": tx.output.iter().map(|txout| json!({
                    "value": txout.value,
                    "scriptPubKey": txout.script_pubkey.to_hex()
                })).collect::<Vec<Value>>(),
            }))
        }
    }

    pub fn transaction_get_merkle(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = sha256d_from_value(params.get(0))?;
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

    pub fn transaction_id_from_pos(&self, params: &[Value]) -> Result<Value> {
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

    pub fn on_chaintip_change(&mut self, chaintip: HeaderEntry) -> Result<Option<Value>> {
        let timer = self
            .stats
            .latency
            .with_label_values(&["chaintip_update"])
            .start_timer();

        if let Some(ref mut last_entry) = self.last_header_entry {
            if *last_entry == chaintip {
                return Ok(None);
            }

            *last_entry = chaintip;
            let hex_header = hex::encode(serialize(last_entry.header()));
            let header = json!({"hex": hex_header, "height": last_entry.height()});
            timer.observe_duration();
            return Ok(Some(json!({
                "jsonrpc": "2.0",
                "method": "blockchain.headers.subscribe",
                "params": [header]})));
        };
        Ok(None)
    }

    pub fn on_scripthash_change(&mut self, scripthash: FullHash) -> Result<Option<Value>> {
        let old_statushash;
        match self.status_hashes.get(&scripthash) {
            Some(statushash) => {
                old_statushash = statushash;
            }
            None => {
                return Ok(None);
            }
        };

        let timer = self
            .stats
            .latency
            .with_label_values(&["statushash_update"])
            .start_timer();

        let timeout = TimeoutTrigger::new(Duration::from_secs(self.rpc_timeout as u64));
        let status = self.query.status(&scripthash, &timeout)?;
        let new_statushash = status.hash().map_or(Value::Null, |h| json!(hex::encode(h)));
        if new_statushash == *old_statushash {
            return Ok(None);
        }
        let notification = Some(json!({
                    "jsonrpc": "2.0",
                    "method": "blockchain.scripthash.subscribe",
                    "params": [scripthash.to_le_hex(), new_statushash]}));
        self.status_hashes.insert(scripthash, new_statushash);
        timer.observe_duration();
        Ok(notification)
    }
}
