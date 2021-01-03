use crate::cache::TransactionCache;
use crate::daemon::Daemon;
use crate::def::COIN;
use crate::errors::*;
use crate::query::header::HeaderQuery;
use bitcoincash::blockdata::script::Script;
use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::encode::{deserialize, serialize};
use bitcoincash::hash_types::{BlockHash, Txid};
use bitcoincash::hashes::hex::ToHex;
use bitcoincash::network::constants::Network;
use bitcoincash::util::address::Payload::{PubkeyHash, ScriptHash};
use bitcoincash::util::address::{Address, AddressType};
use rust_decimal::prelude::*;
use serde_json::Value;
use std::sync::Arc;

///  String returned is intended to be the same as produced by bitcoind
///  GetTxnOutputType
fn get_address_type(script: &Script, network: Network) -> Option<&str> {
    if script.is_op_return() {
        return Some("nulldata");
    }
    let address = Address::from_script(script, network)?;
    let address_type = address.address_type();
    match address_type {
        Some(AddressType::P2pkh) => Some("pubkeyhash"),
        Some(AddressType::P2sh) => Some("scripthash"),
        _ => {
            if !address.is_standard() {
                Some("nonstandard")
            } else {
                None
            }
        }
    }
}

fn get_addresses(script: &Script, network: Network) -> Vec<String> {
    let address = match Address::from_script(script, network) {
        Some(a) => a,
        None => return vec![],
    };

    let cashaddr_network = match network {
        Network::Bitcoin => bitcoincash_addr::Network::Main,
        Network::Testnet => bitcoincash_addr::Network::Test,
        Network::Regtest => bitcoincash_addr::Network::Regtest,
    };

    match address.payload {
        PubkeyHash(pubhash) => {
            let hash = pubhash.as_hash().to_vec();
            let encoded = bitcoincash_addr::Address::new(
                hash,
                bitcoincash_addr::Scheme::CashAddr,
                bitcoincash_addr::HashType::Key,
                cashaddr_network,
            )
            .encode();
            match encoded {
                Ok(addr) => vec![addr],
                _ => vec![],
            }
        }
        ScriptHash(scripthash) => {
            let hash = scripthash.as_hash().to_vec();
            let encoded = bitcoincash_addr::Address::new(
                hash,
                bitcoincash_addr::Scheme::CashAddr,
                bitcoincash_addr::HashType::Script,
                cashaddr_network,
            )
            .encode();
            match encoded {
                Ok(addr) => vec![addr],
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

fn value_from_amount(amount: u64) -> Value {
    if amount == 0 {
        return json!(0.0);
    }
    let satoshis = Decimal::new(amount as i64, 0);
    // rust-decimal crate with feature 'serde-float' should make this work
    // without introducing precision errors
    json!(satoshis.checked_div(Decimal::new(COIN as i64, 0)).unwrap())
}

pub struct TxQuery {
    tx_cache: TransactionCache,
    daemon: Daemon,
    header: Arc<HeaderQuery>,
    duration: Arc<prometheus::HistogramVec>,
    network: Network,
}

impl TxQuery {
    pub fn new(
        tx_cache: TransactionCache,
        daemon: Daemon,
        header: Arc<HeaderQuery>,
        duration: Arc<prometheus::HistogramVec>,
        network: Network,
    ) -> TxQuery {
        TxQuery {
            tx_cache,
            daemon,
            header,
            duration,
            network,
        }
    }

    pub fn get(
        &self,
        txid: &Txid,
        blockhash: Option<&BlockHash>,
        blockheight: Option<u32>,
    ) -> Result<Transaction> {
        let _timer = self.duration.with_label_values(&["load_txn"]).start_timer();
        if let Some(tx) = self.tx_cache.get(txid) {
            return Ok(tx);
        }
        let hash: Option<BlockHash> = match blockhash {
            Some(hash) => Some(*hash),
            None => match self.header.get_by_txid(txid, blockheight) {
                Ok(header) => header.map(|h| *h.hash()),
                Err(_) => None,
            },
        };
        self.load_txn_from_bitcoind(txid, hash.as_ref())
    }

    /// Get an transaction known to be unconfirmed.
    ///
    /// This is slightly faster that `get` as it avoids blockhash lookup. May
    /// or may not return the transaction even if it is confirmed.
    pub fn get_unconfirmed(&self, txid: &Txid) -> Result<Transaction> {
        if let Some(tx) = self.tx_cache.get(txid) {
            Ok(tx)
        } else {
            self.load_txn_from_bitcoind(txid, None)
        }
    }

    pub fn get_verbose(&self, txid: &Txid) -> Result<Value> {
        let header = self.header.get_by_txid(&txid, None)?;
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
                if let Some(best) = self.header.best() {
                    best.height() - header.height()
                } else {
                    0
                }
            }
            None => 0,
        };
        let blockhash = if let Some(h) = header {
            Some(*h.hash())
        } else {
            None
        };
        let tx = self.get(txid, blockhash.as_ref(), None)?;
        let tx_serialized = serialize(&tx);
        Ok(json!({
            "blockhash": blockhash.unwrap_or_default().to_hex(),
            "blocktime": blocktime,
            "height": height,
            "confirmations": confirmations,
            "hash": tx.txid().to_hex(),
            "txid": tx.txid().to_hex(),
            "size": tx_serialized.len(),
            "hex": hex::encode(tx_serialized),
            "locktime": tx.lock_time,
            "time": blocktime,
            "version": tx.version,
            "vin": tx.input.iter().map(|txin| json!({
                // bitcoind adds scriptSig hex as 'coinbase' when the transaction is a coinbase
                "coinbase": if tx.is_coin_base() { Some(txin.script_sig.to_hex()) } else { None },
                "sequence": txin.sequence,
                "txid": txin.previous_output.txid.to_hex(),
                "vout": txin.previous_output.vout,
                "scriptSig": {
                    "asm": txin.script_sig.asm(),
                    "hex": txin.script_sig.to_hex(),
                },
            })).collect::<Vec<Value>>(),
            "vout": tx.output.iter().enumerate().map(|(n, txout)| json!({
                    "value_satoshi": txout.value,
                    "value_coin": value_from_amount(txout.value),
                    "n": n,
                    "scriptPubKey": {
                        "asm": txout.script_pubkey.asm(),
                        "hex": txout.script_pubkey.to_hex(),
                        "type": get_address_type(&txout.script_pubkey, self.network).unwrap_or_default(),
                        "addresses": get_addresses(&txout.script_pubkey, self.network),
                    },
                    })).collect::<Vec<Value>>(),
        }))
    }

    fn load_txn_from_bitcoind(
        &self,
        txid: &Txid,
        blockhash: Option<&BlockHash>,
    ) -> Result<Transaction> {
        let value: Value = self
            .daemon
            .gettransaction_raw(txid, blockhash, /*verbose*/ false)?;
        let value_hex: &str = value.as_str().chain_err(|| "non-string tx")?;
        let serialized_tx = hex::decode(&value_hex).chain_err(|| "non-hex tx")?;
        let tx = deserialize(&serialized_tx).chain_err(|| "failed to parse serialized tx")?;
        self.tx_cache.put(txid, serialized_tx);
        Ok(tx)
    }
}
