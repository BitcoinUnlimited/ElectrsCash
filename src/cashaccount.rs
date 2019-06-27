extern crate scopeguard;
use crate::errors::*;
use crate::store::ReadStore;
use crate::store::Row;
use crate::util::{hash_prefix, Bytes, FullHash, HashPrefix};
use bincode;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin_hashes::sha256d::Hash as Sha256dHash;
use c_fixed_string::CFixedStr;
use cashaccount_sys::{
    cashacc_account_destroy, cashacc_account_init, cashacc_parse_opreturn, CashAccount,
    CASHACC_ERR_MALLOC_FAILED,
};
use crypto::digest::Digest;
use crypto::sha2::Sha256;
use std::ffi::CStr;

fn compute_accountname_hash(accountname: &[u8], blockheight: usize) -> FullHash {
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
    pub fn new(txid: &Sha256dHash, accountname: &[u8], blockheight: usize) -> TxCashAccountRow {
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

    pub fn filter(accountname: &[u8], blockheight: usize) -> Bytes {
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

pub fn index_cashaccount<'a>(txn: &'a Transaction, blockheight: usize) -> Result<Row> {
    let account = unsafe { cashacc_account_init() };
    let _dest = scopeguard::guard((), |_| {
        unsafe { cashacc_account_destroy(account) };
    });

    if !parse_cashaccount(account, txn) {
        return Err("no cashaccount".into());
    }
    let name = unsafe { CStr::from_ptr((*account).name).to_str().unwrap() };
    Ok(TxCashAccountRow::new(
        &txn.txid(),
        name.to_ascii_lowercase().as_bytes(),
        blockheight,
    )
    .to_row())
}

pub fn txids_by_cashaccount(store: &ReadStore, name: &str, height: usize) -> Vec<HashPrefix> {
    store
        .scan(&TxCashAccountRow::filter(
            name.to_ascii_lowercase().as_bytes(),
            height,
        ))
        .iter()
        .map(|row| TxCashAccountRow::from_row(row).txid_prefix)
        .collect()
}

pub fn has_cashaccount(txn: &Transaction, name: &str) -> bool {
    let account = unsafe { cashacc_account_init() };
    let _dest = scopeguard::guard((), |_| {
        unsafe { cashacc_account_destroy(account) };
    });
    if !parse_cashaccount(account, txn) {
        return false;
    }
    let txn_name = unsafe { CStr::from_ptr((*account).name) };
    match txn_name.to_str() {
        Ok(n) => n.to_ascii_lowercase().eq(&name.to_ascii_lowercase()),
        Err(_n) => false,
    }
}
