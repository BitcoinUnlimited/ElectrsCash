use crate::cache::TransactionCache;
use crate::daemon::Daemon;
use crate::errors::*;
use crate::query::header::HeaderQuery;
use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::deserialize;
use bitcoincash::hash_types::{BlockHash, Txid};
use serde_json::Value;
use std::sync::Arc;

pub struct TxQuery {
    tx_cache: TransactionCache,
    daemon: Daemon,
    header: Arc<HeaderQuery>,
    duration: Arc<prometheus::HistogramVec>,
}

impl TxQuery {
    pub fn new(
        tx_cache: TransactionCache,
        daemon: Daemon,
        header: Arc<HeaderQuery>,
        duration: Arc<prometheus::HistogramVec>,
    ) -> TxQuery {
        TxQuery {
            tx_cache,
            daemon,
            header,
            duration,
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
