use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::hash_types::Txid;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound;
use std::sync::Mutex;

use crate::daemon::{Daemon, MempoolEntry};
use crate::errors::*;
use crate::index::index_transaction;
use crate::metrics::Metrics;
use crate::query::tx::TxQuery;
use crate::store::{ReadStore, Row};
use crate::util::Bytes;

const VSIZE_BIN_WIDTH: u32 = 100_000; // in vbytes

/// Fake height value used to signify that a transaction is in the memory pool.
pub const MEMPOOL_HEIGHT: u32 = 0x7FFF_FFFF;

pub enum ConfirmationState {
    Indeterminate,
    Confirmed,
    InMempool,
    UnconfirmedParent,
}

struct MempoolStore {
    map: BTreeMap<Bytes, Vec<Bytes>>,
}

impl MempoolStore {
    fn new() -> MempoolStore {
        MempoolStore {
            map: BTreeMap::new(),
        }
    }

    #[allow(clippy::redundant_closure)]
    fn add(&mut self, tx: &Transaction) {
        let rows = index_transaction(tx, MEMPOOL_HEIGHT as usize, None);
        for row in rows {
            let (key, value) = row.into_pair();
            self.map.entry(key).or_insert_with(|| vec![]).push(value);
        }
    }

    fn remove(&mut self, tx: &Transaction) {
        let rows = index_transaction(tx, MEMPOOL_HEIGHT as usize, None);
        for row in rows {
            let (key, value) = row.into_pair();
            let no_values_left = {
                let values = self
                    .map
                    .get_mut(&key)
                    .unwrap_or_else(|| panic!("missing key {} in mempool", hex::encode(&key)));
                let last_value = values
                    .pop()
                    .unwrap_or_else(|| panic!("no values found for key {}", hex::encode(&key)));
                // TxInRow and TxOutRow have an empty value, TxRow has height=0 as value.
                assert_eq!(
                    value,
                    last_value,
                    "wrong value for key {}: {}",
                    hex::encode(&key),
                    hex::encode(&last_value)
                );
                values.is_empty()
            };
            if no_values_left {
                self.map.remove(&key).unwrap();
            }
        }
    }
}

impl ReadStore for MempoolStore {
    fn get(&self, key: &[u8]) -> Option<Bytes> {
        Some(self.map.get(key)?.last()?.to_vec())
    }
    fn scan(&self, prefix: &[u8]) -> Vec<Row> {
        let range = self
            .map
            .range((Bound::Included(prefix.to_vec()), Bound::Unbounded));
        let mut rows = vec![];
        for (key, values) in range {
            if !key.starts_with(prefix) {
                break;
            }
            if let Some(value) = values.last() {
                rows.push(Row {
                    key: key.to_vec(),
                    value: value.to_vec(),
                });
            }
        }
        rows
    }
}

struct Item {
    tx: Transaction,     // stored for faster retrieval and index removal
    entry: MempoolEntry, // caches mempool fee rates
}

struct Stats {
    count: prometheus::IntGauge,
    update: prometheus::HistogramVec,
    vsize: prometheus::GaugeVec,
    max_fee_rate: Mutex<f32>,
}

impl Stats {
    fn start_timer(&self, step: &str) -> prometheus::HistogramTimer {
        self.update.with_label_values(&[step]).start_timer()
    }

    fn update(&self, entries: &[&MempoolEntry]) {
        let mut bands: Vec<(f32, u32)> = vec![];
        let mut fee_rate = 1.0f32; // [sat/vbyte]
        let mut vsize = 0u32; // vsize of transactions paying <= fee_rate
        for e in entries {
            while fee_rate < e.fee_per_vbyte() {
                bands.push((fee_rate, vsize));
                fee_rate *= 2.0;
            }
            vsize += e.vsize();
        }
        let mut max_fee_rate = self.max_fee_rate.lock().unwrap();
        loop {
            bands.push((fee_rate, vsize));
            if fee_rate < *max_fee_rate {
                fee_rate *= 2.0;
                continue;
            }
            *max_fee_rate = fee_rate;
            break;
        }
        drop(max_fee_rate);
        for (fee_rate, vsize) in bands {
            // labels should be ordered by fee_rate value
            let label = format!("≤{:10.0}", fee_rate);
            self.vsize
                .with_label_values(&[&label])
                .set(f64::from(vsize));
        }
    }
}

pub struct Tracker {
    items: HashMap<Txid, Item>,
    index: MempoolStore,
    histogram: Vec<(f32, u32)>,
    stats: Stats,
}

impl Tracker {
    pub fn new(metrics: &Metrics) -> Tracker {
        Tracker {
            items: HashMap::new(),
            index: MempoolStore::new(),
            histogram: vec![],
            stats: Stats {
                count: metrics.gauge_int(prometheus::Opts::new(
                    "electrscash_mempool_count",
                    "# of mempool transactions",
                )),
                update: metrics.histogram_vec(
                    prometheus::HistogramOpts::new(
                        "electrscash_mempool_update",
                        "Time to update mempool (in seconds)",
                    ),
                    &["step"],
                ),
                vsize: metrics.gauge_float_vec(
                    prometheus::Opts::new(
                        "electrscash_mempool_vsize",
                        "Total vsize of transactions paying at most given fee rate",
                    ),
                    &["fee_rate"],
                ),
                max_fee_rate: Mutex::new(1.0),
            },
        }
    }

    pub fn get_txn(&self, txid: &Txid) -> Option<Transaction> {
        self.items.get(txid).map(|stats| stats.tx.clone())
    }

    pub fn has_txn(&self, txid: &Txid) -> bool {
        self.items.contains_key(txid)
    }

    pub fn get_fee(&self, txid: &Txid) -> Option<u64> {
        self.items.get(txid).map(|stats| stats.entry.fee())
    }

    pub fn contains(&self, txid: &Txid) -> bool {
        self.items.contains_key(txid)
    }

    /// Returns vector of (fee_rate, vsize) pairs, where fee_{n-1} > fee_n and vsize_n is the
    /// total virtual size of mempool transactions with fee in the bin [fee_{n-1}, fee_n].
    /// Note: fee_{-1} is implied to be infinite.
    pub fn fee_histogram(&self) -> &Vec<(f32, u32)> {
        &self.histogram
    }

    pub fn index(&self) -> &dyn ReadStore {
        &self.index
    }

    pub fn update(&mut self, daemon: &Daemon, txquery: &TxQuery) -> Result<HashSet<Txid>> {
        // set of transactions where a change has occurred (either new or removed)
        let mut changed_txs: HashSet<Txid> = HashSet::new();

        let timer = self.stats.start_timer("fetch");
        let new_txids = daemon
            .getmempooltxids()
            .chain_err(|| "failed to update mempool from daemon")?;
        let old_txids = self.items.keys().cloned().collect();
        timer.observe_duration();

        let timer = self.stats.start_timer("add");
        let txids_iter = new_txids.difference(&old_txids);
        let entries = txids_iter.filter_map(|txid| {
            match daemon.getmempoolentry(txid) {
                Ok(entry) => Some((txid, entry)),
                Err(err) => {
                    debug!("no mempool entry {}: {}", txid, err); // e.g. new block or RBF
                    None // ignore this transaction for now
                }
            }
        });
        for (txid, entry) in entries {
            match txquery.get_unconfirmed(txid) {
                Ok(tx) => {
                    assert_eq!(tx.txid(), *txid);
                    self.add(txid, tx, entry);
                    changed_txs.insert(*txid);
                }
                Err(err) => {
                    debug!("failed to get transaction {}: {}", txid, err); // e.g. new block or RBF
                }
            }
        }
        timer.observe_duration();

        let timer = self.stats.start_timer("remove");
        for txid in old_txids.difference(&new_txids) {
            self.remove(txid);
            changed_txs.insert(*txid);
        }
        timer.observe_duration();

        let timer = self.stats.start_timer("fees");
        self.update_fee_histogram();
        timer.observe_duration();

        self.stats.count.set(self.items.len() as i64);
        Ok(changed_txs)
    }

    pub fn tx_confirmation_state(&self, txid: &Txid, height: Option<u32>) -> ConfirmationState {
        if let Some(height) = height {
            if height != MEMPOOL_HEIGHT {
                return ConfirmationState::Confirmed;
            }
        }

        // Check if any of our inputs are unconfirmed
        if let Some(tx) = self.get_txn(txid) {
            for input in tx.input.iter() {
                let prevout = &input.previous_output.txid;
                if self.contains(prevout) {
                    return ConfirmationState::UnconfirmedParent;
                }
            }
            ConfirmationState::InMempool
        } else {
            // We don't see it in our mempool.
            ConfirmationState::Indeterminate
        }
    }

    fn add(&mut self, txid: &Txid, tx: Transaction, entry: MempoolEntry) {
        self.index.add(&tx);
        self.items.insert(*txid, Item { tx, entry });
    }

    fn remove(&mut self, txid: &Txid) {
        let stats = self
            .items
            .remove(txid)
            .unwrap_or_else(|| panic!("missing mempool tx {}", txid));
        self.index.remove(&stats.tx);
    }

    fn update_fee_histogram(&mut self) {
        let mut entries: Vec<&MempoolEntry> = self.items.values().map(|stat| &stat.entry).collect();
        entries.sort_unstable_by(|e1, e2| {
            e1.fee_per_vbyte().partial_cmp(&e2.fee_per_vbyte()).unwrap()
        });
        self.histogram = electrum_fees(&entries);
        self.stats.update(&entries);
    }
}

fn electrum_fees(entries: &[&MempoolEntry]) -> Vec<(f32, u32)> {
    let mut histogram = vec![];
    let mut bin_size = 0;
    let mut last_fee_rate = 0.0;
    let error_margin = 1.192_092_9e-7; // TODO: Replace with f32::EPSILON
    for e in entries.iter().rev() {
        let fee_rate = e.fee_per_vbyte();
        if bin_size > VSIZE_BIN_WIDTH && (last_fee_rate - fee_rate).abs() > error_margin {
            // vsize of transactions paying >= e.fee_per_vbyte()
            histogram.push((last_fee_rate, bin_size));
            bin_size = 0;
        }
        last_fee_rate = fee_rate;
        bin_size += e.vsize();
    }
    if bin_size > 0 {
        histogram.push((last_fee_rate, bin_size));
    }
    histogram
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_fakestore() {
        use crate::daemon::MempoolEntry;
        use crate::mempool::electrum_fees;

        let entries = [
            // (fee: u64, vsize: u32)
            &MempoolEntry::new(1_000, 1_000),
            &MempoolEntry::new(10_000, 10_000),
            &MempoolEntry::new(50_000, 50_000),
            &MempoolEntry::new(120_000, 60_000),
            &MempoolEntry::new(210_000, 70_000),
            &MempoolEntry::new(320_000, 80_000),
        ];
        assert_eq!(
            electrum_fees(&entries[..]),
            vec![(3.0, 150_000), (1.0, 121_000)]
        );
    }
}
