use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::{deserialize, serialize};
use bitcoin_hashes::hex::ToHex;
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use bitcoin_hashes::Hash;
use crypto::digest::Digest;
use crypto::sha2::Sha256;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::app::App;
use crate::cache::TransactionCache;
use crate::cashaccount::{has_cashaccount, txids_by_cashaccount};
use crate::errors::*;
use crate::index::{compute_script_hash, TxInRow, TxOutRow, TxRow};
use crate::mempool::{Tracker, MEMPOOL_HEIGHT};
use crate::metrics::{HistogramOpts, HistogramVec, Metrics};
use crate::store::{ReadStore, Row};
use crate::util::{FullHash, HashPrefix, HeaderEntry};

enum ConfirmationState {
    Confirmed,
    InMempool,
    UnconfirmedParent,
}

pub struct FundingOutput {
    pub txn_id: Sha256dHash,
    pub height: u32,
    pub output_index: usize,
    pub value: u64,
    state: ConfirmationState,
}

type OutPoint = (Sha256dHash, usize); // (txid, output_index)

struct SpendingInput {
    txn_id: Sha256dHash,
    height: u32,
    funding_output: OutPoint,
    value: u64,
    state: ConfirmationState,
}

pub struct Status {
    confirmed: (Vec<FundingOutput>, Vec<SpendingInput>),
    mempool: (Vec<FundingOutput>, Vec<SpendingInput>),
}

fn calc_balance((funding, spending): &(Vec<FundingOutput>, Vec<SpendingInput>)) -> i64 {
    let funded: u64 = funding.iter().map(|output| output.value).sum();
    let spent: u64 = spending.iter().map(|input| input.value).sum();
    funded as i64 - spent as i64
}

impl Status {
    fn funding(&self) -> impl Iterator<Item = &FundingOutput> {
        self.confirmed.0.iter().chain(self.mempool.0.iter())
    }

    fn spending(&self) -> impl Iterator<Item = &SpendingInput> {
        self.confirmed.1.iter().chain(self.mempool.1.iter())
    }

    pub fn confirmed_balance(&self) -> i64 {
        calc_balance(&self.confirmed)
    }

    pub fn mempool_balance(&self) -> i64 {
        calc_balance(&self.mempool)
    }

    pub fn history(&self) -> Vec<(i32, Sha256dHash)> {
        let mut txns_map = HashMap::<Sha256dHash, i32>::new();
        for f in self.funding() {
            let height: i32 = match f.state {
                ConfirmationState::Confirmed => f.height as i32,
                ConfirmationState::InMempool => 0,
                ConfirmationState::UnconfirmedParent => -1,
            };

            txns_map.insert(f.txn_id, height);
        }
        for s in self.spending() {
            let height: i32 = match s.state {
                ConfirmationState::Confirmed => s.height as i32,
                ConfirmationState::InMempool => 0,
                ConfirmationState::UnconfirmedParent => -1,
            };
            txns_map.insert(s.txn_id, height as i32);
        }
        let mut txns: Vec<(i32, Sha256dHash)> =
            txns_map.into_iter().map(|item| (item.1, item.0)).collect();
        txns.sort_unstable_by(|a, b| {
            if a.0 == b.0 {
                // Order by little endian tx hash if height is the same,
                // in most cases, this order is the same as on the blockchain.
                return b.1.cmp(&a.1);
            }
            if a.0 > 0 && b.0 > 0 {
                return a.0.cmp(&b.0);
            }

            // mempool txs should be sorted last, so add to it a large number
            let mut a_height = a.0;
            let mut b_height = b.0;
            if a_height <= 0 {
                a_height = 0xEEEEEE + a_height.abs();
            }
            if b_height <= 0 {
                b_height = 0xEEEEEE + b_height.abs();
            }
            return a_height.cmp(&b_height);
        });
        txns
    }

    pub fn unspent(&self) -> Vec<&FundingOutput> {
        let mut outputs_map = HashMap::<OutPoint, &FundingOutput>::new();
        for f in self.funding() {
            outputs_map.insert((f.txn_id, f.output_index), f);
        }
        for s in self.spending() {
            if outputs_map.remove(&s.funding_output).is_none() {
                warn!("failed to remove {:?}", s.funding_output);
            }
        }
        let mut outputs = outputs_map
            .into_iter()
            .map(|item| item.1) // a reference to unspent output
            .collect::<Vec<&FundingOutput>>();
        outputs.sort_unstable_by_key(|out| out.height);
        outputs
    }

    pub fn hash(&self) -> Option<FullHash> {
        let txns = self.history();
        if txns.is_empty() {
            None
        } else {
            let mut hash = FullHash::default();
            let mut sha2 = Sha256::new();
            for (height, txn_id) in txns {
                let part = format!("{}:{}:", txn_id.to_hex(), height);
                sha2.input(part.as_bytes());
            }
            sha2.result(&mut hash);
            Some(hash)
        }
    }
}

struct TxnHeight {
    txn: Transaction,
    height: u32,
}

fn merklize(left: Sha256dHash, right: Sha256dHash) -> Sha256dHash {
    let data = [&left[..], &right[..]].concat();
    Sha256dHash::hash(&data)
}

fn create_merkle_branch_and_root(
    mut hashes: Vec<Sha256dHash>,
    mut index: usize,
) -> (Vec<Sha256dHash>, Sha256dHash) {
    let mut merkle = vec![];
    while hashes.len() > 1 {
        if hashes.len() % 2 != 0 {
            let last = *hashes.last().unwrap();
            hashes.push(last);
        }
        index = if index % 2 == 0 { index + 1 } else { index - 1 };
        merkle.push(hashes[index]);
        index /= 2;
        hashes = hashes
            .chunks(2)
            .map(|pair| merklize(pair[0], pair[1]))
            .collect()
    }
    (merkle, hashes[0])
}

// TODO: the functions below can be part of ReadStore.
fn txrow_by_txid(store: &dyn ReadStore, txid: &Sha256dHash) -> Option<TxRow> {
    let key = TxRow::filter_full(&txid);
    let value = store.get(&key)?;
    Some(TxRow::from_row(&Row { key, value }))
}

fn txrows_by_prefix(store: &dyn ReadStore, txid_prefix: HashPrefix) -> Vec<TxRow> {
    store
        .scan(&TxRow::filter_prefix(txid_prefix))
        .iter()
        .map(|row| TxRow::from_row(row))
        .collect()
}

fn txids_by_script_hash(store: &dyn ReadStore, script_hash: &[u8]) -> Vec<HashPrefix> {
    store
        .scan(&TxOutRow::filter(script_hash))
        .iter()
        .map(|row| TxOutRow::from_row(row).txid_prefix)
        .collect()
}

fn txids_by_funding_output(
    store: &dyn ReadStore,
    txn_id: &Sha256dHash,
    output_index: usize,
) -> Vec<HashPrefix> {
    store
        .scan(&TxInRow::filter(&txn_id, output_index))
        .iter()
        .map(|row| TxInRow::from_row(row).txid_prefix)
        .collect()
}

pub struct Query {
    app: Arc<App>,
    tracker: RwLock<Tracker>,
    tx_cache: TransactionCache,
    txid_limit: usize,
    duration: HistogramVec,
}

impl Query {
    pub fn new(
        app: Arc<App>,
        metrics: &Metrics,
        tx_cache: TransactionCache,
        txid_limit: usize,
    ) -> Arc<Query> {
        Arc::new(Query {
            app,
            tracker: RwLock::new(Tracker::new(metrics)),
            tx_cache,
            txid_limit,
            duration: metrics.histogram_vec(
                HistogramOpts::new(
                    "electrs_query_duration",
                    "Time to update mempool (in seconds)",
                ),
                &["type"],
            ),
        })
    }

    fn load_txns_by_prefix(
        &self,
        store: &dyn ReadStore,
        prefixes: Vec<HashPrefix>,
    ) -> Result<Vec<TxnHeight>> {
        let mut txns = vec![];
        for txid_prefix in prefixes {
            for tx_row in txrows_by_prefix(store, txid_prefix) {
                let txid: Sha256dHash = deserialize(&tx_row.key.txid).unwrap();
                let blockhash = self.lookup_confirmed_blockhash(&txid, Some(tx_row.height))?;
                let txn = self.load_txn(&txid, blockhash)?;
                txns.push(TxnHeight {
                    txn,
                    height: tx_row.height,
                })
            }
        }
        Ok(txns)
    }

    fn find_spending_input(
        &self,
        store: &dyn ReadStore,
        funding: &FundingOutput,
    ) -> Result<Option<SpendingInput>> {
        let spending_txns: Vec<TxnHeight> = self.load_txns_by_prefix(
            store,
            txids_by_funding_output(store, &funding.txn_id, funding.output_index),
        )?;
        let mut spending_inputs = vec![];
        for t in &spending_txns {
            for input in t.txn.input.iter() {
                if input.previous_output.txid == funding.txn_id
                    && input.previous_output.vout == funding.output_index as u32
                {
                    spending_inputs.push(SpendingInput {
                        txn_id: t.txn.txid(),
                        height: t.height,
                        funding_output: (funding.txn_id, funding.output_index),
                        value: funding.value,
                        state: self.check_confirmation_state(t),
                    })
                }
            }
        }
        assert!(spending_inputs.len() <= 1);
        Ok(if spending_inputs.len() == 1 {
            Some(spending_inputs.remove(0))
        } else {
            None
        })
    }

    fn check_confirmation_state(&self, t: &TxnHeight) -> ConfirmationState {
        if t.height != MEMPOOL_HEIGHT {
            return ConfirmationState::Confirmed;
        }

        // Check if any of our inputs are unconfirmed
        for input in t.txn.input.iter() {
            let prevout = &input.previous_output.txid;
            if self.tracker.read().unwrap().contains(prevout) {
                return ConfirmationState::UnconfirmedParent;
            }
        }
        ConfirmationState::InMempool
    }

    fn find_funding_outputs(&self, t: &TxnHeight, script_hash: &[u8]) -> Vec<FundingOutput> {
        let mut result = vec![];
        let txn_id = t.txn.txid();
        for (index, output) in t.txn.output.iter().enumerate() {
            if compute_script_hash(&output.script_pubkey[..]) == script_hash {
                result.push(FundingOutput {
                    txn_id,
                    height: t.height,
                    output_index: index,
                    value: output.value,
                    state: self.check_confirmation_state(t),
                })
            }
        }
        result
    }

    fn confirmed_status(
        &self,
        script_hash: &[u8],
    ) -> Result<(Vec<FundingOutput>, Vec<SpendingInput>)> {
        let mut funding = vec![];
        let mut spending = vec![];
        let read_store = self.app.read_store();
        let txid_prefixes = txids_by_script_hash(read_store, script_hash);
        // if the limit is enabled
        if self.txid_limit > 0 && txid_prefixes.len() > self.txid_limit {
            bail!(
                "{}+ transactions found, query may take a long time",
                txid_prefixes.len()
            );
        }
        for t in self.load_txns_by_prefix(read_store, txid_prefixes)? {
            funding.extend(self.find_funding_outputs(&t, script_hash));
        }
        for funding_output in &funding {
            if let Some(spent) = self.find_spending_input(read_store, &funding_output)? {
                spending.push(spent);
            }
        }
        Ok((funding, spending))
    }

    fn mempool_status(
        &self,
        script_hash: &[u8],
        confirmed_funding: &[FundingOutput],
    ) -> Result<(Vec<FundingOutput>, Vec<SpendingInput>)> {
        let mut funding = vec![];
        let mut spending = vec![];
        let tracker = self.tracker.read().unwrap();
        let txid_prefixes = txids_by_script_hash(tracker.index(), script_hash);
        for t in self.load_txns_by_prefix(tracker.index(), txid_prefixes)? {
            funding.extend(self.find_funding_outputs(&t, script_hash));
        }
        // // TODO: dedup outputs (somehow) both confirmed and in mempool (e.g. reorg?)
        for funding_output in funding.iter().chain(confirmed_funding.iter()) {
            if let Some(spent) = self.find_spending_input(tracker.index(), &funding_output)? {
                spending.push(spent);
            }
        }
        Ok((funding, spending))
    }

    pub fn status(&self, script_hash: &[u8]) -> Result<Status> {
        let timer = self
            .duration
            .with_label_values(&["confirmed_status"])
            .start_timer();
        let confirmed = self
            .confirmed_status(script_hash)
            .chain_err(|| "failed to get confirmed status")?;
        timer.observe_duration();

        let timer = self
            .duration
            .with_label_values(&["mempool_status"])
            .start_timer();
        let mempool = self
            .mempool_status(script_hash, &confirmed.0)
            .chain_err(|| "failed to get mempool status")?;
        timer.observe_duration();

        Ok(Status { confirmed, mempool })
    }

    fn lookup_confirmed_blockhash(
        &self,
        tx_hash: &Sha256dHash,
        block_height: Option<u32>,
    ) -> Result<Option<Sha256dHash>> {
        if self.tracker.read().unwrap().get_txn(&tx_hash).is_some() {
            return Ok(None);
        }
        // Lookup in confirmed transactions' index
        let height = match block_height {
            Some(height) => {
                if height == MEMPOOL_HEIGHT {
                    return Ok(None);
                }
                height
            }
            None => {
                txrow_by_txid(self.app.read_store(), &tx_hash)
                    .chain_err(|| format!("not indexed tx {}", tx_hash))?
                    .height
            }
        };
        let header = self
            .app
            .index()
            .get_header(height as usize)
            .chain_err(|| format!("missing header at height {}", height))?;
        Ok(Some(*header.hash()))
    }

    /// Load transaction by ID, attempt to lookup blockhash locally.
    pub fn load_txn_with_blockhashlookup(
        &self,
        txid: &Sha256dHash,
        blockhash: Option<Sha256dHash>,
    ) -> Result<Transaction> {
        if let Some(tx) = self.tracker.read().unwrap().get_txn(&txid) {
            return Ok(tx);
        }
        let hash: Option<Sha256dHash> = match blockhash {
            Some(hash) => Some(hash),
            None => match self.lookup_confirmed_blockhash(txid, None) {
                Ok(hash) => hash,
                Err(_) => None,
            },
        };
        self.load_txn(txid, hash)
    }

    pub fn load_txn(
        &self,
        txid: &Sha256dHash,
        blockhash: Option<Sha256dHash>,
    ) -> Result<Transaction> {
        let _timer = self.duration.with_label_values(&["load_txn"]).start_timer();
        self.tx_cache.get_or_else(&txid, || {
            let value: Value = self
                .app
                .daemon()
                .gettransaction_raw(txid, blockhash, /*verbose*/ false)?;
            let value_hex: &str = value.as_str().chain_err(|| "non-string tx")?;
            hex::decode(&value_hex).chain_err(|| "non-hex tx")
        })
    }

    // Public API for transaction retrieval (for Electrum RPC)
    pub fn get_transaction(&self, tx_hash: &Sha256dHash, verbose: bool) -> Result<Value> {
        let _timer = self
            .duration
            .with_label_values(&["get_transaction"])
            .start_timer();
        let blockhash = self.lookup_confirmed_blockhash(tx_hash, /*block_height*/ None)?;
        self.app
            .daemon()
            .gettransaction_raw(tx_hash, blockhash, verbose)
    }

    pub fn get_headers(&self, heights: &[usize]) -> Vec<HeaderEntry> {
        let _timer = self
            .duration
            .with_label_values(&["get_headers"])
            .start_timer();
        let index = self.app.index();
        heights
            .iter()
            .filter_map(|height| index.get_header(*height))
            .collect()
    }

    pub fn get_best_header(&self) -> Result<HeaderEntry> {
        let last_header = self.app.index().best_header();
        Ok(last_header.chain_err(|| "no headers indexed")?.clone())
    }

    pub fn getblocktxids(&self, blockhash: &Sha256dHash) -> Result<Vec<Sha256dHash>> {
        self.app.daemon().getblocktxids(blockhash)
    }

    pub fn get_merkle_proof(
        &self,
        tx_hash: &Sha256dHash,
        height: usize,
    ) -> Result<(Vec<Sha256dHash>, usize)> {
        let header_entry = self
            .app
            .index()
            .get_header(height)
            .chain_err(|| format!("missing block #{}", height))?;
        let txids = self.app.daemon().getblocktxids(&header_entry.hash())?;
        let pos = txids
            .iter()
            .position(|txid| txid == tx_hash)
            .chain_err(|| format!("missing txid {}", tx_hash))?;
        let (branch, _root) = create_merkle_branch_and_root(txids, pos);
        Ok((branch, pos))
    }

    pub fn get_header_merkle_proof(
        &self,
        height: usize,
        cp_height: usize,
    ) -> Result<(Vec<Sha256dHash>, Sha256dHash)> {
        if cp_height < height {
            bail!("cp_height #{} < height #{}", cp_height, height);
        }

        let best_height = self.get_best_header()?.height();
        if best_height < cp_height {
            bail!(
                "cp_height #{} above best block height #{}",
                cp_height,
                best_height
            );
        }

        let heights: Vec<usize> = (0..=cp_height).collect();
        let header_hashes: Vec<Sha256dHash> = self
            .get_headers(&heights)
            .into_iter()
            .map(|h| *h.hash())
            .collect();
        assert_eq!(header_hashes.len(), heights.len());
        Ok(create_merkle_branch_and_root(header_hashes, height))
    }

    pub fn get_id_from_pos(
        &self,
        height: usize,
        tx_pos: usize,
        want_merkle: bool,
    ) -> Result<(Sha256dHash, Vec<Sha256dHash>)> {
        let header_entry = self
            .app
            .index()
            .get_header(height)
            .chain_err(|| format!("missing block #{}", height))?;

        let txids = self.app.daemon().getblocktxids(header_entry.hash())?;
        let txid = *txids
            .get(tx_pos)
            .chain_err(|| format!("No tx in position #{} in block #{}", tx_pos, height))?;

        let branch = if want_merkle {
            create_merkle_branch_and_root(txids, tx_pos).0
        } else {
            vec![]
        };
        Ok((txid, branch))
    }

    pub fn broadcast(&self, txn: &Transaction) -> Result<Sha256dHash> {
        self.app.daemon().broadcast(txn)
    }

    pub fn update_mempool(&self) -> Result<HashSet<Sha256dHash>> {
        let _timer = self
            .duration
            .with_label_values(&["update_mempool"])
            .start_timer();
        self.tracker.write().unwrap().update(self.app.daemon())
    }

    /// Returns [vsize, fee_rate] pairs (measured in vbytes and satoshis).
    pub fn get_fee_histogram(&self) -> Vec<(f32, u32)> {
        self.tracker.read().unwrap().fee_histogram().clone()
    }

    // Fee rate [BTC/kB] to be confirmed in `blocks` from now.
    pub fn estimate_fee(&self, blocks: usize) -> f32 {
        let mut total_vsize = 0u32;
        let mut last_fee_rate = 0.0;
        let blocks_in_vbytes = (blocks * 1_000_000) as u32; // assume ~1MB blocks
        for (fee_rate, vsize) in self.tracker.read().unwrap().fee_histogram() {
            last_fee_rate = *fee_rate;
            total_vsize += vsize;
            if total_vsize >= blocks_in_vbytes {
                break; // under-estimate the fee rate a bit
            }
        }
        last_fee_rate * 1e-5 // [BTC/kB] = 10^5 [sat/B]
    }

    pub fn get_banner(&self) -> Result<String> {
        self.app.get_banner()
    }

    pub fn get_cashaccount_txs(&self, name: &str, height: u32) -> Result<Value> {
        let cashaccount_txns: Vec<TxnHeight> = self.load_txns_by_prefix(
            self.app.read_store(),
            txids_by_cashaccount(self.app.read_store(), name, height),
        )?;

        // filter on name in case of txid prefix collision
        let cashaccount_txns = cashaccount_txns
            .iter()
            .filter(|txn| has_cashaccount(&txn.txn, name));

        #[derive(Serialize, Deserialize, Debug)]
        struct AccountTx {
            tx: String,
            height: u32,
            blockhash: String,
        };

        let header = self
            .app
            .index()
            .get_header(height as usize)
            .chain_err(|| format!("missing header at height {}", height))?;
        let blockhash = *header.hash();

        let cashaccount_txns: Vec<AccountTx> = cashaccount_txns
            .map(|txn| AccountTx {
                tx: hex::encode(&serialize(&txn.txn)),
                height: txn.height,
                blockhash: blockhash.to_hex(),
            })
            .collect();

        Ok(json!(cashaccount_txns))
    }
}
