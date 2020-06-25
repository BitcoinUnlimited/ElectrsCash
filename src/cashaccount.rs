use crate::mempool::MEMPOOL_HEIGHT;
use crate::scripthash::FullHash;
use crate::store::ReadStore;
use crate::store::Row;
use crate::util::{hash_prefix, Bytes, HashPrefix};
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::hash_types::Txid;
use c_fixed_string::CFixedStr;
use cashaccount_sys::{
    cashacc_account_destroy, cashacc_account_init, cashacc_parse_opreturn, CashAccount,
    CASHACC_ERR_MALLOC_FAILED,
};
use crypto::digest::Digest;
use crypto::sha2::Sha256;
use std::ffi::CStr;

fn compute_accountname_hash(accountname: &[u8], blockheight: u32) -> FullHash {
    let mut hash = FullHash::default();
    let mut sha2 = Sha256::new();
    sha2.input(accountname);
    sha2.input(&blockheight.to_be_bytes());
    sha2.result(&mut hash);
    hash
}

#[derive(Serialize, Deserialize)]
pub struct TxCashAccountKey {
    code: u8,
    accout_hash_prefix: HashPrefix,
}

#[derive(Serialize, Deserialize)]
pub struct TxCashAccountRow {
    key: TxCashAccountKey,
    pub txid_prefix: HashPrefix,
}

impl TxCashAccountRow {
    pub fn new(txid: &Txid, accountname: &[u8], blockheight: u32) -> TxCashAccountRow {
        TxCashAccountRow {
            key: TxCashAccountKey {
                code: b'C',
                accout_hash_prefix: hash_prefix(&compute_accountname_hash(
                    accountname,
                    blockheight,
                )),
            },
            txid_prefix: hash_prefix(&txid[..]),
        }
    }

    pub fn filter(accountname: &[u8], blockheight: u32) -> Bytes {
        bincode::serialize(&TxCashAccountKey {
            code: b'C',
            accout_hash_prefix: hash_prefix(&compute_accountname_hash(accountname, blockheight)),
        })
        .unwrap()
    }

    pub fn to_row(&self) -> Row {
        Row {
            key: bincode::serialize(&self).unwrap(),
            value: vec![],
        }
    }

    pub fn from_row(row: &Row) -> TxCashAccountRow {
        bincode::deserialize(&row.key).expect("failed to parse TxCashAccountRow")
    }
}

pub fn txids_by_cashaccount(store: &dyn ReadStore, name: &str, height: u32) -> Vec<HashPrefix> {
    store
        .scan(&TxCashAccountRow::filter(
            name.to_ascii_lowercase().as_bytes(),
            height,
        ))
        .iter()
        .map(|row| TxCashAccountRow::from_row(row).txid_prefix)
        .collect()
}

fn parse_cashaccount(account: *mut CashAccount, txn: &Transaction) -> bool {
    let mut opreturn_found = false;
    let mut cashaccount_found = false;
    for out in txn.output.iter() {
        if !out.script_pubkey.is_op_return() {
            continue;
        }
        if opreturn_found {
            // CashAccount transaction can only contain 1 OP_RETURN output.
            // We've now seen a second one.
            return false;
        }

        // OP_RETURN found. Parse to see if it contains a cashaccount.
        opreturn_found = true;
        let script: &Script = &out.script_pubkey;
        let bytes = CFixedStr::from_bytes(script.as_bytes());
        let rc =
            unsafe { cashacc_parse_opreturn(bytes.as_ptr(), script.len() as u32, false, account) };

        assert!(rc != CASHACC_ERR_MALLOC_FAILED);
        if rc < 1 {
            // not valid cashaccount, or no payload found.
            return false;
        }
        cashaccount_found = true;
    }
    cashaccount_found
}

const CASHACCOUNT_INDEX_DISABLED: u32 = 0;

pub fn is_valid_cashaccount_height(activation_height: u32, height: u32) -> bool {
    height >= activation_height && height != MEMPOOL_HEIGHT && height != CASHACCOUNT_INDEX_DISABLED
}

pub struct CashAccountParser {
    account: *mut CashAccount,
    activation_height: u32,
}

impl CashAccountParser {
    pub fn new(activation_height: Option<u32>) -> CashAccountParser {
        CashAccountParser {
            account: unsafe { cashacc_account_init() },
            activation_height: activation_height.unwrap_or(0),
        }
    }

    pub fn has_cashaccount(&self, txn: &Transaction, name: &str) -> bool {
        if !parse_cashaccount(self.account, txn) {
            return false;
        }
        let txn_name = unsafe { CStr::from_ptr((*self.account).name) };
        match txn_name.to_str() {
            Ok(n) => n.to_ascii_lowercase().eq(&name.to_ascii_lowercase()),
            Err(_n) => false,
        }
    }

    pub fn index_cashaccount<'a>(&self, txn: &'a Transaction, blockheight: u32) -> Option<Row> {
        if !is_valid_cashaccount_height(self.activation_height, blockheight) {
            return None;
        }

        if !parse_cashaccount(self.account, txn) {
            return None;
        }
        let name = unsafe { CStr::from_ptr((*self.account).name).to_str().unwrap() };
        Some(
            TxCashAccountRow::new(
                &txn.txid(),
                name.to_ascii_lowercase().as_bytes(),
                blockheight,
            )
            .to_row(),
        )
    }
}

impl Drop for CashAccountParser {
    fn drop(&mut self) {
        unsafe { cashacc_account_destroy(self.account) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_is_valid_cashaccount_height() {
        let activation = 1000;
        assert!(!is_valid_cashaccount_height(
            activation,
            CASHACCOUNT_INDEX_DISABLED
        ));
        assert!(!is_valid_cashaccount_height(activation, MEMPOOL_HEIGHT));
        assert!(is_valid_cashaccount_height(activation, MEMPOOL_HEIGHT - 1));

        assert!(!is_valid_cashaccount_height(activation, activation - 1));
        assert!(is_valid_cashaccount_height(activation, activation));
        assert!(is_valid_cashaccount_height(activation, activation + 1));
    }
}
