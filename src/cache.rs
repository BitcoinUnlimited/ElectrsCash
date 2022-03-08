use crate::errors::*;
use crate::metrics::Metrics;
use crate::rndcache::RndCache;

use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::encode::deserialize;
use bitcoincash::hash_types::{BlockHash, Txid};
use std::sync::{Mutex, RwLock};

pub struct BlockTxIDsCache {
    map: Mutex<RndCache<BlockHash, Vec<Txid>>>,
}

impl BlockTxIDsCache {
    pub fn new(bytes_capacity: u64, metrics: &Metrics) -> BlockTxIDsCache {
        let lookups = metrics.counter_int_vec(
            prometheus::Opts::new(
                "electrscash_cache_blocktxids_lookups",
                "# of cache lookups in the blocktxids cache",
            ),
            &["type"],
        );
        let churn = metrics.counter_int_vec(
            prometheus::Opts::new(
                "electrscash_cache_blocktxids_churn",
                "# of insertions and evictions from the blocktxids cache",
            ),
            &["type"],
        );
        let size = metrics.gauge_int(prometheus::Opts::new(
            "electrscash_cache_blocktxids_size",
            "Size of the blockstxid cache [bytes]",
        ));
        let entries = metrics.gauge_int(prometheus::Opts::new(
            "electrscash_cache_blocktxids_entries",
            "# of entries in the blockstxid cache",
        ));
        BlockTxIDsCache {
            map: Mutex::new(RndCache::new(bytes_capacity, lookups, churn, size, entries)),
        }
    }

    pub fn get_or_else<F>(&self, blockhash: &BlockHash, load_txids_func: F) -> Result<Vec<Txid>>
    where
        F: FnOnce() -> Result<Vec<Txid>>,
    {
        if let Some(txids) = self.map.lock().unwrap().get(blockhash) {
            return Ok(txids.clone());
        }

        let txids = load_txids_func()?;
        let mut cache_copy = txids.clone();
        cache_copy.shrink_to_fit();
        let size = cache_copy.capacity();
        self.map
            .lock()
            .unwrap()
            .put(*blockhash, cache_copy, size as u64);
        Ok(txids)
    }
}

pub struct TransactionCache {
    // Store serialized transaction (should use less RAM).
    map: RwLock<RndCache<Txid, Vec<u8>>>,
}

impl TransactionCache {
    pub fn new(bytes_capacity: u64, metrics: &Metrics) -> TransactionCache {
        let lookups = metrics.counter_int_vec(
            prometheus::Opts::new(
                "electrscash_cache_tx_lookups",
                "# of cache lookups in the transaction cache",
            ),
            &["type"],
        );
        let churn = metrics.counter_int_vec(
            prometheus::Opts::new(
                "electrscash_cache_tx_churn",
                "# of insertions and evictions from the transaction cache",
            ),
            &["type"],
        );
        let size = metrics.gauge_int(prometheus::Opts::new(
            "electrscash_cache_tx_size",
            "Size of the transaction cache [bytes]",
        ));
        let entries = metrics.gauge_int(prometheus::Opts::new(
            "electrscash_cache_tx_entries",
            "# of entries in the transaction cache",
        ));
        TransactionCache {
            map: RwLock::new(RndCache::new(bytes_capacity, lookups, churn, size, entries)),
        }
    }

    pub fn get(&self, txid: &Txid) -> Option<Transaction> {
        if let Some(serialized_txn) = self.map.read().unwrap().get(txid) {
            if let Ok(tx) = deserialize(serialized_txn) {
                return Some(tx);
            } else {
                trace!("failed to parse a cached tx");
            }
        }
        None
    }

    pub fn put(&self, txid: &Txid, mut serialized_tx: Vec<u8>) {
        serialized_tx.shrink_to_fit();
        let size = serialized_tx.capacity();
        self.map
            .write()
            .unwrap()
            .put(*txid, serialized_tx, size as u64);
    }
}
