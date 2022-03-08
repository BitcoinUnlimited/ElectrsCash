use crate::errors::*;
use crate::index::{TxInRow, TxOutRow, TxRow};
use crate::mempool::{ConfirmationState, Tracker, MEMPOOL_HEIGHT};
use crate::query::primitives::{FundingOutput, SpendingInput};
use crate::query::tx::TxQuery;
use crate::scripthash::compute_script_hash;
use crate::store::{ReadStore, Row};
use crate::timeout::TimeoutTrigger;
use crate::util::{hash_prefix, HashPrefix};
use bitcoincash::blockdata::transaction::OutPoint;
use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::consensus::encode::deserialize;
use bitcoincash::hash_types::Txid;
use genawaiter::{sync::gen, yield_};

// TODO: the functions below can be part of ReadStore.
pub fn txrow_by_txid(store: &dyn ReadStore, txid: &Txid) -> Option<TxRow> {
    let key = TxRow::filter_full(txid);
    let value = store.get(&key)?;
    Some(TxRow::from_row(&Row { key, value }))
}

pub fn txrows_by_prefix(store: &dyn ReadStore, txid_prefix: HashPrefix) -> Vec<TxRow> {
    store
        .scan(&TxRow::filter_prefix(txid_prefix))
        .iter()
        .map(TxRow::from_row)
        .collect()
}

pub fn txoutrows_by_script_hash(store: &dyn ReadStore, script_hash: &[u8]) -> Vec<TxOutRow> {
    store
        .scan(&TxOutRow::filter(script_hash))
        .iter()
        .map(TxOutRow::from_row)
        .collect()
}

pub fn txids_by_funding_output(store: &dyn ReadStore, prevout: &OutPoint) -> Vec<HashPrefix> {
    store
        .scan(&TxInRow::filter(prevout))
        .iter()
        .map(|row| TxInRow::from_row(row).txid_prefix)
        .collect()
}

/// Mempool parameter is optional if it's known that the transaction is
/// confired.
pub fn txoutrow_to_fundingoutput(
    store: &dyn ReadStore,
    txoutrow: &TxOutRow,
    mempool: Option<&Tracker>,
    txquery: &TxQuery,
    timeout: &TimeoutTrigger,
) -> Result<FundingOutput> {
    let txrow = lookup_tx_by_outrow(store, txoutrow, txquery, timeout)?;
    let txid = txrow.get_txid();

    Ok(FundingOutput {
        funding_output: OutPoint::new(txid, txoutrow.get_output_index()),
        height: txrow.height,
        value: txoutrow.get_output_value(),
        state: confirmation_state(mempool, &txid, txrow.height),
    })
}

/// Lookup txrow using txid prefix, filter on output when there are
/// multiple matches.
fn lookup_tx_by_outrow(
    store: &dyn ReadStore,
    txout: &TxOutRow,
    txquery: &TxQuery,
    timeout: &TimeoutTrigger,
) -> Result<TxRow> {
    let mut txrows = txrows_by_prefix(store, txout.txid_prefix);
    if txrows.len() == 1 {
        return Ok(txrows.remove(0));
    }
    let output_index = txout.get_output_index();
    for txrow in txrows {
        timeout.check()?;
        let tx = txquery.get(&txrow.get_txid(), None, Some(txrow.height))?;
        if txn_has_output(&tx, output_index, txout.key.script_hash_prefix) {
            return Ok(txrow);
        }
    }
    Err("tx not in store".into())
}

fn txn_has_output(txn: &Transaction, n: u32, scripthash_prefix: HashPrefix) -> bool {
    let n = n as usize;
    if txn.output.len() - 1 < n {
        return false;
    }
    let hash = compute_script_hash(&txn.output[n].script_pubkey[..]);
    hash_prefix(&hash) == scripthash_prefix
}

fn confirmation_state(mempool: Option<&Tracker>, txid: &Txid, height: u32) -> ConfirmationState {
    // If mempool parameter is not passed, this implies that it is known
    // that the transaction is confirmed.
    if mempool.is_none() || height != MEMPOOL_HEIGHT {
        return ConfirmationState::Confirmed;
    }
    let mempool = mempool.unwrap();
    mempool.tx_confirmation_state(txid, Some(height))
}

pub fn find_spending_input(
    store: &dyn ReadStore,
    funding: &FundingOutput,
    mempool: Option<&Tracker>,
    txquery: &TxQuery,
    timeout: &TimeoutTrigger,
) -> Result<Option<SpendingInput>> {
    let spending_txns = txids_by_funding_output(store, &funding.funding_output);

    if spending_txns.len() == 1 {
        let spender_txid = &spending_txns[0];
        let txrows = txrows_by_prefix(store, *spender_txid);
        if txrows.len() == 1 {
            // One match, assume it's correct to avoid load_txn lookup.
            let txid = txrows[0].get_txid();
            return Ok(Some(SpendingInput {
                txn_id: txid,
                height: txrows[0].height,
                funding_output: funding.funding_output,
                value: funding.value,
                state: confirmation_state(mempool, &txid, txrows[0].height),
            }));
        }
    }
    if spending_txns.is_empty() {
        return Ok(None);
    }

    // Ambiguity, fetch from bitcoind to verify
    for (height, tx) in load_txns_by_prefix(store, spending_txns, txquery) {
        let tx = tx?;
        for input in tx.input.iter() {
            if input.previous_output != funding.funding_output {
                continue;
            }
            let txid = tx.txid();
            let state = confirmation_state(mempool, &txid, height);
            return Ok(Some(SpendingInput {
                txn_id: txid,
                height,
                funding_output: funding.funding_output,
                value: funding.value,
                state,
            }));
        }
        timeout.check()?;
    }
    Ok(None)
}

// TODO: Combine with above method
pub fn get_tx_spending_prevout(
    store: &dyn ReadStore,
    txquery: &TxQuery,
    timeout: &TimeoutTrigger,
    prevout: &OutPoint,
) -> Result<
    Option<(
        Transaction,
        u32, /* input index */
        u32, /* confirmation height */
    )>,
> {
    for txid_prefix in store
        .scan(&TxInRow::filter(prevout))
        .iter()
        .map(|row| TxInRow::from_row(row).txid_prefix)
    {
        for txrow in store
            .scan(&TxRow::filter_prefix(txid_prefix))
            .iter()
            .map(TxRow::from_row)
        {
            let tx = txquery.get(&txrow.get_txid(), None, Some(txrow.height))?;
            for (n, input) in tx.input.iter().enumerate() {
                if input.previous_output != *prevout {
                    continue;
                }
                let height = if txrow.height == MEMPOOL_HEIGHT {
                    0
                } else {
                    txrow.height
                };
                return Ok(Some((tx, n as u32, height)));
            }
            timeout.check()?;
        }
    }
    Ok(None)
}

pub fn load_txns_by_prefix<'a>(
    store: &'a dyn ReadStore,
    prefixes: Vec<HashPrefix>,
    txquery: &'a TxQuery,
) -> impl Iterator<Item = (u32, Result<Transaction>)> + 'a {
    gen!({
        for txid_prefix in prefixes {
            for tx_row in txrows_by_prefix(store, txid_prefix) {
                let txid: Txid = deserialize(&tx_row.key.txid).unwrap();
                let txn = txquery.get(&txid, None, Some(tx_row.height));
                yield_!((tx_row.height, txn));
            }
        }
    })
    .into_iter()
}
