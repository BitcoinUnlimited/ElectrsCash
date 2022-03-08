use crate::doslimit::ConnectionLimits;
use crate::errors::*;
use crate::query::Query;
use crate::rpc::parseutil::{
    bool_from_value_or, hash_from_value, rpc_arg_error, scripthash_from_value, str_from_value,
    usize_from_value, usize_from_value_or,
};
use crate::rpc::rpcstats::RpcStats;
use crate::rpc::scripthash::{get_balance, get_first_use, get_history, get_mempool, listunspent};
use crate::scripthash::addr_to_scripthash;
use crate::scripthash::{compute_script_hash, FullHash, ToLeHex};
use crate::timeout::TimeoutTrigger;
use crate::util::HeaderEntry;
use bitcoincash::blockdata::transaction::OutPoint;
use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::encode::{deserialize, serialize};
use bitcoincash::hash_types::Txid;
use bitcoincash::hashes::hex::ToHex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Subscription {
    statushash: Option<FullHash>,
    alias: Option<String>,
}

pub struct BlockchainRpc {
    query: Arc<Query>,
    stats: Arc<RpcStats>,
    subscriptions: Mutex<HashMap<FullHash /* scripthash */, Subscription>>,
    last_header_entry: Mutex<Option<HeaderEntry>>,
    relayfee: f64,
    doslimits: ConnectionLimits,

    /* Resource tracking */
    alias_bytes_used: AtomicUsize,
}

impl BlockchainRpc {
    pub fn new(
        query: Arc<Query>,
        stats: Arc<RpcStats>,
        relayfee: f64,
        doslimits: ConnectionLimits,
    ) -> BlockchainRpc {
        BlockchainRpc {
            query,
            stats,
            subscriptions: Mutex::new(HashMap::new()),
            last_header_entry: Mutex::new(None), // disable header subscription for now
            relayfee,
            doslimits,
            alias_bytes_used: AtomicUsize::new(0),
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

    pub fn address_get_mempool(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        get_mempool(&self.query, &scripthash, timeout)
    }

    pub fn address_get_scripthash(&self, params: &[Value]) -> Result<Value> {
        let scripthash = addr_to_scripthash(&str_from_value(params.get(0), "address")?)?;
        Ok(json!(scripthash.to_le_hex()))
    }

    pub fn address_listunspent(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        listunspent(&*self.query, &scripthash, timeout)
    }

    pub fn address_subscribe(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        self.remove_subscription(&scripthash);

        self.doslimits
            .check_subscriptions(self.get_num_subscriptions() as u32 + 1)?;

        self.doslimits
            .check_alias_usage(self.alias_bytes_used.load(Ordering::Relaxed) + addr.len())?;

        let statushash = self.query.status(&scripthash, timeout)?.hash();
        let result = statushash.map_or(Value::Null, |h| json!(hex::encode(h)));

        // We don't hold a lock on alias usage, so we could exceed limit here.
        // That's OK, it doesn't need to be a hard limit.
        self.alias_bytes_used
            .fetch_add(addr.len(), Ordering::Relaxed);
        self.subscriptions.lock().unwrap().insert(
            scripthash,
            Subscription {
                statushash,
                alias: Some(addr),
            },
        );
        self.stats.subscriptions.inc();
        Ok(result)
    }

    pub fn address_unsubscribe(&self, params: &[Value]) -> Result<Value> {
        let addr = str_from_value(params.get(0), "address")?;
        let scripthash = addr_to_scripthash(&addr)?;
        Ok(json!(self.remove_subscription(&scripthash)))
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

    pub fn headers_subscribe(&self) -> Result<Value> {
        let entry = self.query.get_best_header()?;
        let hex_header = hex::encode(serialize(entry.header()));
        let result = json!({"hex": hex_header, "height": entry.height()});
        let mut last_entry = self.last_header_entry.lock().unwrap();
        *last_entry = Some(entry);
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

    pub fn scripthash_get_mempool(
        &self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        get_mempool(&self.query, &scripthash, timeout)
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
        &self,
        params: &[Value],
        timeout: &TimeoutTrigger,
    ) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        self.remove_subscription(&scripthash);

        self.doslimits
            .check_subscriptions(self.get_num_subscriptions() as u32 + 1)?;

        let statushash = self.query.status(&scripthash, timeout)?.hash();
        let result = statushash.map_or(Value::Null, |h| json!(hex::encode(h)));
        self.subscriptions.lock().unwrap().insert(
            scripthash,
            Subscription {
                statushash,
                alias: None,
            },
        );
        self.stats.subscriptions.inc();
        Ok(result)
    }

    pub fn scripthash_unsubscribe(&self, params: &[Value]) -> Result<Value> {
        let scripthash = scripthash_from_value(params.get(0))?;
        Ok(json!(self.remove_subscription(&scripthash)))
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
        let tx_hash = hash_from_value::<Txid>(params.get(0))?;
        let verbose = match params.get(1) {
            Some(value) => value.as_bool().chain_err(|| "non-bool verbose value")?,
            None => false,
        };
        if !verbose {
            let tx = self.query.tx().get(&tx_hash, None, None)?;
            Ok(json!(hex::encode(serialize(&tx))))
        } else {
            self.query.tx().get_verbose(&tx_hash)
        }
    }

    pub fn transaction_get_confirmed_blockhash(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = hash_from_value(params.get(0)).chain_err(|| "bad tx_hash")?;
        self.query.get_confirmed_blockhash(&tx_hash)
    }

    pub fn transaction_get_merkle(&self, params: &[Value]) -> Result<Value> {
        let tx_hash = hash_from_value::<Txid>(params.get(0))?;
        let height = if params.get(1).is_some() {
            usize_from_value(params.get(1), "height")
        } else {
            let header = self.query.header().get_by_txid(&tx_hash, None)?;
            match header {
                Some(header) => Ok(header.height()),
                None => Err(rpc_arg_error(&format!(
                    "Transaction '{}' is not confirmed in a block",
                    tx_hash.to_hex()
                ))
                .into()),
            }
        }?;

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

    pub fn utxo_get(&self, params: &[Value], timeout: &TimeoutTrigger) -> Result<Value> {
        let txid = hash_from_value::<Txid>(params.get(0))?;
        let out_n = usize_from_value(params.get(1), "out_n")?;
        if out_n > u32::MAX as usize {
            return Err(rpc_arg_error(&format!(
                "Too large value for out_n parameter ({} > {})",
                out_n,
                u32::MAX
            ))
            .into());
        }

        // We want to provide the utxo amount regardless of if it's spent or
        // unspent.
        let utxo_creation_tx = self.query.tx().get(&txid, None, None)?;
        timeout.check()?;

        let utxo = match utxo_creation_tx.output.get(out_n) {
            Some(utxo) => utxo,
            None => {
                bail!(rpc_invalid_params(format!(
                    "out_n {} does not exist on tx {}, the transaction has {} outputs",
                    out_n,
                    txid,
                    utxo_creation_tx.output.len()
                )));
            }
        };

        // Fetch the spending transaction (if the utxo is spent).
        let spend = self
            .query
            .get_tx_spending_prevout(&OutPoint::new(txid, out_n as u32), timeout)?;

        let status = if spend.is_some() { "spent" } else { "unspent" };

        let spent_json = match spend {
            Some((tx, input_index, height)) => {
                json!({
                    "tx_hash": Some(tx.txid().to_string()),
                    "tx_pos": Some(input_index),
                    "height": Some(height),
                })
            }
            None => {
                json!({
                    "tx_hash": None::<String>,
                    "tx_pos": None::<u32>,
                    "height": None::<i64>,
                })
            }
        };

        let utxo_confirmation_height = self.query.tx().get_confirmation_height(&txid);
        let utxo_scripthash = compute_script_hash(&utxo.script_pubkey[..]);

        Ok(json!({
            "status": status,
            "amount": utxo.value,
            "scripthash": utxo_scripthash.to_le_hex(),
            "height": utxo_confirmation_height,
            "spent": spent_json,
        }))
    }

    pub fn on_chaintip_change(&self, chaintip: HeaderEntry) -> Result<Option<Value>> {
        let timer = self
            .stats
            .latency
            .with_label_values(&["chaintip_update"])
            .start_timer();

        let mut last_entry = self.last_header_entry.lock().unwrap();
        if last_entry.is_none() {
            return Ok(None);
        }
        if last_entry.as_ref() == Some(&chaintip) {
            return Ok(None);
        }

        *last_entry = Some(chaintip);
        let hex_header = hex::encode(serialize(last_entry.as_ref().unwrap().header()));
        let header = json!({"hex": hex_header, "height": last_entry.as_ref().unwrap().height()});
        timer.observe_duration();
        Ok(Some(json!({
            "jsonrpc": "2.0",
            "method": "blockchain.headers.subscribe",
            "params": [header]})))
    }

    pub fn on_scripthash_change(&self, scripthash: FullHash) -> Result<Option<Value>> {
        let old_statushash: Option<FullHash>;
        let subscription_name: String;
        let method: &str;

        let mut subscriptions = self.subscriptions.lock().unwrap();

        match subscriptions.get(&scripthash) {
            Some(subscription) => {
                old_statushash = subscription.statushash;
                if let Some(alias) = &subscription.alias {
                    subscription_name = alias.clone();
                    method = "blockchain.address.subscribe";
                } else {
                    subscription_name = scripthash.to_le_hex();
                    method = "blockchain.scripthash.subscribe";
                }
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

        let timeout = TimeoutTrigger::new(Duration::from_secs(self.doslimits.rpc_timeout as u64));
        let status = self.query.status(&scripthash, &timeout)?;
        let new_statushash = status.hash();
        if new_statushash == old_statushash {
            return Ok(None);
        }
        let new_statushash_hex = status.hash().map_or(Value::Null, |h| json!(hex::encode(h)));
        let notification = Some(json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": [subscription_name, new_statushash_hex]}));
        subscriptions.get_mut(&scripthash).unwrap().statushash = new_statushash;
        timer.observe_duration();
        Ok(notification)
    }

    pub fn get_num_subscriptions(&self) -> i64 {
        self.subscriptions.lock().unwrap().len() as i64
    }

    fn remove_subscription(&self, scripthash: &FullHash) -> bool {
        let removed = self.subscriptions.lock().unwrap().remove(scripthash);
        match removed {
            Some(subscription) => {
                if let Some(alias) = subscription.alias {
                    self.alias_bytes_used
                        .fetch_sub(alias.len(), Ordering::Relaxed);
                }
                self.stats.subscriptions.dec();
                true
            }
            None => false,
        }
    }
}
