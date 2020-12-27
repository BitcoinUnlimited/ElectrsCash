use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::encode::serialize;
use bitcoincash::hash_types::{BlockHash, TxMerkleNode, Txid};
use bitcoincash::hashes::hex::ToHex;
use bitcoincash::hashes::sha256d::Hash as Sha256dHash;
use bitcoincash::hashes::Hash;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::app::App;
use crate::cache::TransactionCache;
use crate::cashaccount::{txids_by_cashaccount, CashAccountParser};
use crate::errors::*;
use crate::index::TxRow;
use crate::mempool::{ConfirmationState, Tracker};
use crate::metrics::Metrics;
use crate::query::confirmed::ConfirmedQuery;
use crate::query::header::HeaderQuery;
use crate::query::primitives::{FundingOutput, OutPoint, SpendingInput};
use crate::query::queryutil::{
    load_txns_by_prefix, txoutrows_by_script_hash, txrows_by_prefix, TxnHeight,
};
use crate::query::tx::TxQuery;
use crate::query::unconfirmed::UnconfirmedQuery;
use crate::scripthash::{compute_script_hash, FullHash};
use crate::timeout::TimeoutTrigger;
use crate::util::HeaderEntry;

pub mod confirmed;
pub mod header;
pub mod primitives;
pub mod queryutil;
pub mod tx;
pub mod unconfirmed;

pub struct Status {
    confirmed: (Vec<FundingOutput>, Vec<SpendingInput>),
    mempool: (Vec<FundingOutput>, Vec<SpendingInput>),
    txn_fees: HashMap<Txid, u64>,
}

fn calc_balance((funding, spending): &(Vec<FundingOutput>, Vec<SpendingInput>)) -> i64 {
    let funded: u64 = funding.iter().map(|output| output.value).sum();
    let spent: u64 = spending.iter().map(|input| input.value).sum();
    funded as i64 - spent as i64
}

pub struct HistoryItem {
    height: i32,
    tx_hash: Txid,
    fee: Option<u64>, // need to be set only for unconfirmed transactions (i.e. height <= 0)
}

impl HistoryItem {
    pub fn to_json(&self) -> Value {
        let mut result = json!({ "height": self.height, "tx_hash": self.tx_hash.to_hex()});
        self.fee.map(|f| {
            result
                .as_object_mut()
                .unwrap()
                .insert("fee".to_string(), json!(f))
        });
        result
    }
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

    pub fn history(&self) -> Vec<HistoryItem> {
        let mut txns_map = HashMap::<Txid, i32>::new();
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
        let mut items: Vec<HistoryItem> = txns_map
            .into_iter()
            .map(|item| HistoryItem {
                height: item.1,
                tx_hash: item.0,
                fee: self.txn_fees.get(&item.0).cloned(),
            })
            .collect();

        items.sort_unstable_by(|a, b| {
            if a.height == b.height {
                // Order by little endian tx hash if height is the same,
                // in most cases, this order is the same as on the blockchain.
                return b.tx_hash.cmp(&a.tx_hash);
            }
            if a.height > 0 && b.height > 0 {
                return a.height.cmp(&b.height);
            }

            // mempool txs should be sorted last, so add to it a large number
            let mut a_height = a.height;
            let mut b_height = b.height;
            if a_height <= 0 {
                a_height = 0xEE_EEEE + a_height.abs();
            }
            if b_height <= 0 {
                b_height = 0xEE_EEEE + b_height.abs();
            }
            a_height.cmp(&b_height)
        });
        items
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
            let mut sha2 = Sha256::new();
            for item in txns {
                let part = format!("{}:{}:", item.tx_hash.to_hex(), item.height);
                sha2.update(part.as_bytes());
            }
            Some(sha2.finalize().into())
        }
    }
}

fn merklize<T: Hash>(left: T, right: T) -> T {
    let data = [&left[..], &right[..]].concat();
    <T as Hash>::hash(&data)
}

fn create_merkle_branch_and_root<T: Hash>(mut hashes: Vec<T>, mut index: usize) -> (Vec<T>, T) {
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

pub struct Query {
    app: Arc<App>,
    tracker: RwLock<Tracker>,
    duration: Arc<prometheus::HistogramVec>,
    confirmed: ConfirmedQuery,
    unconfirmed: UnconfirmedQuery,
    tx: Arc<TxQuery>,
    header: Arc<HeaderQuery>,
}

impl Query {
    pub fn new(app: Arc<App>, metrics: &Metrics, tx_cache: TransactionCache) -> Result<Arc<Query>> {
        let daemon = app.daemon().reconnect()?;
        let duration = Arc::new(metrics.histogram_vec(
            prometheus::HistogramOpts::new(
                "electrscash_query_duration",
                "Request duration (in seconds)",
            ),
            &["type"],
        ));
        let header = Arc::new(HeaderQuery::new(app.clone()));
        let tx = Arc::new(TxQuery::new(
            tx_cache,
            daemon,
            header.clone(),
            duration.clone(),
        ));
        let confirmed = ConfirmedQuery::new(tx.clone(), duration.clone());
        let unconfirmed = UnconfirmedQuery::new(tx.clone(), duration.clone());
        Ok(Arc::new(Query {
            app,
            tracker: RwLock::new(Tracker::new(metrics)),
            duration,
            confirmed,
            unconfirmed,
            tx,
            header,
        }))
    }

    pub fn status_mempool(
        &self,
        scripthash: &FullHash,
        timeout: &TimeoutTrigger,
    ) -> Result<Status> {
        let store = self.app.read_store();
        let confirmed_funding = self
            .confirmed
            .get_funding(store, scripthash, &*self.tx, timeout)
            .chain_err(|| "failed to get confirmed funding status")?;

        let tracker = self.tracker.read().unwrap();
        let unconfirmed_funding = self
            .unconfirmed
            .get_funding(&tracker, scripthash, timeout)
            .chain_err(|| "failed to get unconfirmed spending status")?;

        let unconfirmed_spending = self
            .unconfirmed
            .get_spending(&tracker, &confirmed_funding, &unconfirmed_funding, timeout)
            .chain_err(|| "failed to get unconfirmed spending status")?;

        let txn_fees =
            self.unconfirmed
                .get_tx_fees(&tracker, &unconfirmed_funding, &unconfirmed_spending);
        let confirmed = (vec![], vec![]);
        let mempool = (unconfirmed_funding, unconfirmed_spending);

        Ok(Status {
            confirmed,
            mempool,
            txn_fees,
        })
    }

    pub fn status(&self, scripthash: &FullHash, timeout: &TimeoutTrigger) -> Result<Status> {
        let store = self.app.read_store();
        let confirmed_funding = self
            .confirmed
            .get_funding(store, scripthash, &*self.tx, timeout)
            .chain_err(|| "failed to get confirmed funding status")?;

        let confirmed_spending = self
            .confirmed
            .get_spending(store, &confirmed_funding, timeout)
            .chain_err(|| "failed to get confirmed spending status")?;

        let tracker = self.tracker.read().unwrap();
        let unconfirmed_funding = self
            .unconfirmed
            .get_funding(&tracker, scripthash, timeout)
            .chain_err(|| "failed to get unconfirmed spending status")?;

        let unconfirmed_spending = self
            .unconfirmed
            .get_spending(&tracker, &confirmed_funding, &unconfirmed_funding, timeout)
            .chain_err(|| "failed to get unconfirmed spending status")?;

        let txn_fees =
            self.unconfirmed
                .get_tx_fees(&tracker, &unconfirmed_funding, &unconfirmed_spending);
        let confirmed = (confirmed_funding, confirmed_spending);
        let mempool = (unconfirmed_funding, unconfirmed_spending);

        Ok(Status {
            confirmed,
            mempool,
            txn_fees,
        })
    }

    pub fn get_confirmed_blockhash(&self, tx_hash: &Txid) -> Result<Value> {
        let header = self.header.get_by_txid(tx_hash, None)?;
        if header.is_none() {
            bail!("tx {} is unconfirmed or does not exist", tx_hash);
        }
        let header = header.unwrap();
        Ok(json!({
            "block_hash": header.hash(),
            "block_height": header.height()
        }))
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
        Ok(last_header.chain_err(|| "no headers indexed")?)
    }

    pub fn getblocktxids(&self, blockhash: &BlockHash) -> Result<Vec<Txid>> {
        self.app.daemon().getblocktxids(blockhash)
    }

    pub fn get_merkle_proof(
        &self,
        tx_hash: &Txid,
        height: usize,
    ) -> Result<(Vec<TxMerkleNode>, usize)> {
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
        let tx_nodes: Vec<TxMerkleNode> = txids
            .into_iter()
            .map(|txid| TxMerkleNode::from_inner(txid.into_inner()))
            .collect();
        let (branch, _root) = create_merkle_branch_and_root(tx_nodes, pos);
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
        let header_hashes: Vec<BlockHash> = self
            .get_headers(&heights)
            .into_iter()
            .map(|h| *h.hash())
            .collect();
        let merkle_nodes: Vec<Sha256dHash> = header_hashes
            .iter()
            .map(|block_hash| Sha256dHash::from_inner(block_hash.into_inner()))
            .collect();
        assert_eq!(header_hashes.len(), heights.len());
        Ok(create_merkle_branch_and_root(merkle_nodes, height))
    }

    pub fn get_id_from_pos(
        &self,
        height: usize,
        tx_pos: usize,
        want_merkle: bool,
    ) -> Result<(Txid, Vec<TxMerkleNode>)> {
        let header_entry = self
            .app
            .index()
            .get_header(height)
            .chain_err(|| format!("missing block #{}", height))?;

        let txids = self.app.daemon().getblocktxids(header_entry.hash())?;
        let txid = *txids
            .get(tx_pos)
            .chain_err(|| format!("No tx in position #{} in block #{}", tx_pos, height))?;

        let tx_nodes = txids
            .into_iter()
            .map(|txid| TxMerkleNode::from_inner(txid.into_inner()))
            .collect();

        let branch = if want_merkle {
            create_merkle_branch_and_root(tx_nodes, tx_pos).0
        } else {
            vec![]
        };
        Ok((txid, branch))
    }

    pub fn broadcast(&self, txn: &Transaction) -> Result<Txid> {
        self.app.daemon().broadcast(txn)
    }

    pub fn update_mempool(&self) -> Result<HashSet<Txid>> {
        let _timer = self
            .duration
            .with_label_values(&["update_mempool"])
            .start_timer();
        self.tracker
            .write()
            .unwrap()
            .update(self.app.daemon(), self.tx())
    }

    /// Returns [vsize, fee_rate] pairs (measured in vbytes and satoshis).
    pub fn get_fee_histogram(&self) -> Vec<(f32, u32)> {
        self.tracker.read().unwrap().fee_histogram().clone()
    }

    // Fee rate [BTC/kB] to be confirmed in `blocks` from now.
    pub fn estimate_fee(&self, blocks: usize) -> f64 {
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
        (last_fee_rate as f64) * 1e-5 // [BTC/kB] = 10^5 [sat/B]
    }

    pub fn get_banner(&self) -> Result<String> {
        self.app.get_banner()
    }

    pub fn get_cashaccount_txs(&self, name: &str, height: u32) -> Result<Value> {
        let cashaccount_txns: Vec<TxnHeight> = load_txns_by_prefix(
            self.app.read_store(),
            txids_by_cashaccount(self.app.read_store(), name, height),
            &self.tx,
        )?;

        // filter on name in case of txid prefix collision
        let parser = CashAccountParser::new(None);
        let cashaccount_txns = cashaccount_txns
            .iter()
            .filter(|txn| parser.has_cashaccount(&txn.txn, name));

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

    /// Find first outputs to scripthash
    pub fn scripthash_first_use(&self, scripthash: &FullHash) -> Result<(u32, Txid)> {
        let get_tx = |store| {
            let rows = txoutrows_by_script_hash(store, scripthash);
            let mut txs: Vec<TxRow> = rows
                .iter()
                .map(|p| txrows_by_prefix(store, p.txid_prefix))
                .flatten()
                .collect();

            txs.sort_unstable_by(|a, b| a.height.cmp(&b.height));

            for txrow in txs.drain(..) {
                // verify that tx contains scripthash as output
                let txid = Txid::from_slice(&txrow.key.txid[..]).expect("invalid txid");
                let tx = self.tx.get(&txid, None, Some(txrow.height))?;

                for o in tx.output.iter() {
                    if compute_script_hash(&o.script_pubkey[..]) == *scripthash {
                        return Ok((txrow.height, txid));
                    }
                }
            }
            Ok((0, Txid::default()))
        };

        // Look at blockchain first
        let tx = get_tx(self.app.read_store())?;
        if tx.0 != 0 {
            return Ok(tx);
        }

        // No match in the blockchain, try the mempool also.
        let tracker = self.tracker.read().unwrap();
        get_tx(tracker.index())
    }

    pub fn get_relayfee(&self) -> Result<f64> {
        self.app.daemon().get_relayfee()
    }

    pub fn tx(&self) -> &TxQuery {
        &self.tx
    }

    pub fn header(&self) -> &HeaderQuery {
        &self.header
    }
}
