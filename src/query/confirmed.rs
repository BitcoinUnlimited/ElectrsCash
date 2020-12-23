use crate::errors::*;
use crate::query::primitives::{FundingOutput, SpendingInput};
use crate::query::queryutil::{
    find_spending_input, txoutrow_to_fundingoutput, txoutrows_by_script_hash,
};
use crate::query::tx::TxQuery;
use crate::scripthash::FullHash;
use crate::store::ReadStore;
use crate::timeout::TimeoutTrigger;
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
            .iter()
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

        let mut spending = vec![];

        for funding_output in confirmed_funding {
            timeout.check()?;
            if let Some(spent) =
                find_spending_input(read_store, &funding_output, None, &*self.txquery, timeout)?
            {
                spending.push(spent);
            }
        }
        timer.observe_duration();
        Ok(spending)
    }
}
