use crate::mempool::ConfirmationState;
use bitcoincash::blockdata::transaction::OutPoint;
use bitcoincash::hash_types::Txid;

pub struct FundingOutput {
    pub funding_output: OutPoint,
    pub height: u32,
    pub value: u64,
    pub state: ConfirmationState,
}

pub struct SpendingInput {
    pub txn_id: Txid,
    pub height: u32,
    pub funding_output: OutPoint,
    pub value: u64,
    pub state: ConfirmationState,
}
