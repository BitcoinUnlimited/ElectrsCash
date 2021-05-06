use crate::errors::*;
use crate::mempool::Tracker;
use crate::query::primitives::{FundingOutput, SpendingInput};
use crate::query::queryutil::{
    find_spending_input, get_tx_spending_prevout, txoutrow_to_fundingoutput,
    txoutrows_by_script_hash,
};
use crate::query::tx::TxQuery;
use crate::scripthash::FullHash;
use crate::timeout::TimeoutTrigger;
use bitcoincash::blockdata::transaction::OutPoint;
use bitcoincash::blockdata::transaction::Transaction;
use bitcoincash::hash_types::Txid;
use std::collections::HashMap;
use std::sync::Arc;

pub struct UnconfirmedQuery {
    txquery: Arc<TxQuery>,
    duration: Arc<prometheus::HistogramVec>,
}

impl UnconfirmedQuery {
    pub fn new(txquery: Arc<TxQuery>, duration: Arc<prometheus::HistogramVec>) -> UnconfirmedQuery {
        UnconfirmedQuery { txquery, duration }
    }

    pub fn get_funding(
        &self,
        tracker: &Tracker,
        scripthash: &FullHash,
        timeout: &TimeoutTrigger,
    ) -> Result<Vec<FundingOutput>> {
        let timer = self
            .duration
            .with_label_values(&["mempool_status_funding"])
            .start_timer();
        let funding = txoutrows_by_script_hash(tracker.index(), scripthash);
        let funding: Result<Vec<FundingOutput>> = funding
            .iter()
            .map(|outrow| {
                txoutrow_to_fundingoutput(
                    tracker.index(),
                    outrow,
                    Some(tracker),
                    &*self.txquery,
                    timeout,
                )
            })
            .collect();
        timer.observe_duration();
        funding
    }

    /// Get unconfirmed use of input spending from scripthash destination.
    ///
    /// unconfirmed_funding is obtain by calling self.get_funding,
    /// confirmed_funding is obtained by calling ConfirmedQuery::get_funding
    pub fn get_spending(
        &self,
        tracker: &Tracker,
        unconfirmed_funding: &[FundingOutput],
        confirmed_funding: &[FundingOutput],
        timeout: &TimeoutTrigger,
    ) -> Result<Vec<SpendingInput>> {
        let timer = self
            .duration
            .with_label_values(&["mempool_status_spending"])
            .start_timer();
        let mut spending = vec![];

        for funding_output in unconfirmed_funding.iter().chain(confirmed_funding.iter()) {
            timeout.check()?;
            if let Some(spent) = find_spending_input(
                tracker.index(),
                &funding_output,
                Some(tracker),
                &self.txquery,
                timeout,
            )? {
                spending.push(spent);
            }
        }
        timer.observe_duration();
        Ok(spending)
    }

    /// Calculate fees for unconfirmed mempool transactions.
    pub fn get_tx_fees(
        &self,
        tracker: &Tracker,
        funding: &[FundingOutput],
        spending: &[SpendingInput],
    ) -> HashMap<Txid, u64> {
        let mut txn_fees = HashMap::new();
        for mempool_txid in funding
            .iter()
            .map(|f| f.funding_output.txid)
            .chain(spending.iter().map(|s| s.txn_id))
        {
            tracker
                .get_fee(&mempool_txid)
                .map(|fee| txn_fees.insert(mempool_txid, fee));
        }
        txn_fees
    }

    pub fn get_tx_spending_prevout(
        &self,
        tracker: &Tracker,
        timeout: &TimeoutTrigger,
        prevout: &OutPoint,
    ) -> Result<
        Option<(
            Transaction,
            u32, /* input index */
            u32, /* confirmation height */
        )>,
    > {
        // TODO: Check if has unconfirmed parent
        get_tx_spending_prevout(tracker.index(), &*self.txquery, timeout, prevout)
    }
}
