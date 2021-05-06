use crate::errors::*;
use crate::query::primitives::{FundingOutput, SpendingInput};
use crate::query::queryutil::{
    find_spending_input, get_tx_spending_prevout, txoutrow_to_fundingoutput,
    txoutrows_by_script_hash,
};
use crate::query::tx::TxQuery;
use crate::scripthash::FullHash;
use crate::store::ReadStore;
use crate::timeout::TimeoutTrigger;
use bitcoincash::blockdata::transaction::OutPoint;
use bitcoincash::blockdata::transaction::Transaction;
use rayon::prelude::*;
use std::sync::Arc;

pub struct ConfirmedQuery {
    txquery: Arc<TxQuery>,
    duration: Arc<prometheus::HistogramVec>,
}

impl ConfirmedQuery {
    pub fn new(txquery: Arc<TxQuery>, duration: Arc<prometheus::HistogramVec>) -> ConfirmedQuery {
        ConfirmedQuery { txquery, duration }
    }

    /// Query for confirmed outputs that funding scripthash.
    pub fn get_funding(
        &self,
        read_store: &dyn ReadStore,
        scripthash: &FullHash,
        txquery: &TxQuery,
        timeout: &TimeoutTrigger,
    ) -> Result<Vec<FundingOutput>> {
        let timer = self
            .duration
            .with_label_values(&["confirmed_status_funding"])
            .start_timer();
        let funding = txoutrows_by_script_hash(read_store, scripthash);
        timeout.check()?;
        let funding = funding
            .par_iter()
            .map(|outrow| txoutrow_to_fundingoutput(read_store, outrow, None, txquery, timeout))
            .collect();
        timer.observe_duration();
        funding
    }

    /// Query for confirmed inputs that have been spent from scripthash.
    ///
    /// This requires list of transactions funding the scripthash, obtained
    /// with get_funding.
    pub fn get_spending(
        &self,
        read_store: &dyn ReadStore,
        confirmed_funding: &[FundingOutput],
        timeout: &TimeoutTrigger,
    ) -> Result<Vec<SpendingInput>> {
        let timer = self
            .duration
            .with_label_values(&["confirmed_status_spending"])
            .start_timer();

        let spending: Result<Vec<Option<SpendingInput>>> = confirmed_funding
            .par_iter()
            .map(|funding_output| {
                timeout.check().and_then(|_| {
                    find_spending_input(read_store, &funding_output, None, &*self.txquery, timeout)
                })
            })
            .collect();
        let spending = spending?;
        let spending: Vec<SpendingInput> = spending.into_iter().filter_map(|s| s).collect();
        timer.observe_duration();
        Ok(spending)
    }

    pub fn get_tx_spending_prevout(
        &self,
        read_store: &dyn ReadStore,
        timeout: &TimeoutTrigger,
        prevout: &OutPoint,
    ) -> Result<
        Option<(
            Transaction,
            u32, /* input index */
            u32, /* confirmation height */
        )>,
    > {
        get_tx_spending_prevout(read_store, &*self.txquery, timeout, prevout)
    }
}
