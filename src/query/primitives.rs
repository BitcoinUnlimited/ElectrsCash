use crate::mempool::ConfirmationState;
use bitcoincash::hash_types::Txid;

pub struct FundingOutput {
    pub txn_id: Txid,
    pub height: u32,
    pub output_index: usize,
    pub value: u64,
    pub state: ConfirmationState,
}

pub type OutPoint = (Txid, usize); // (txid, output_index)

pub struct SpendingInput {
    pub txn_id: Txid,
    pub height: u32,
    pub funding_output: OutPoint,
    pub value: u64,
    pub state: ConfirmationState,
}
