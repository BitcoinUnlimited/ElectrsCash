use crate::errors::*;
use crate::mempool::MEMPOOL_HEIGHT;
use crate::query::FundingOutput;
use crate::query::{Query, Status};
use crate::scripthash::{FullHash, ToLEHex};
use crate::timeout::TimeoutTrigger;
use bitcoin::hash_types::BlockHash;
use bitcoin_hashes::hex::ToHex;
use serde_json::Value;

fn unspent_to_json(out: &FundingOutput) -> Value {
    json!({
        "height": if out.height == MEMPOOL_HEIGHT { 0 } else { out.height },
        "tx_pos": out.output_index,
        "tx_hash": out.txn_id.to_hex(),
        "value": out.value,
    })
}

fn unspent_from_status(status: &Status) -> Value {
    json!(Value::Array(
        status
            .unspent()
            .into_iter()
            .map(|out| unspent_to_json(out))
            .collect()
    ))
}

pub fn get_balance(
    query: &Query,
    scripthash: &FullHash,
    timeout: &TimeoutTrigger,
) -> Result<Value> {
    let status = query.status(&scripthash, timeout)?;
    Ok(json!({
        "confirmed": status.confirmed_balance(),
        "unconfirmed": status.mempool_balance()
    }))
}

pub fn get_first_use(query: &Query, scripthash: &FullHash) -> Result<Value> {
    let firstuse = query.scripthash_first_use(scripthash)?;
    if firstuse.0 == 0 {
        return Err(ErrorKind::RpcError(
            RpcErrorCode::NotFound,
            format!("scripthash '{}' not found", scripthash.to_le_hex()),
        )
        .into());
    }
    let blockhash = if firstuse.0 == MEMPOOL_HEIGHT {
        BlockHash::default()
    } else {
        let h = query.get_headers(&[firstuse.0 as usize]);
        if h.is_empty() {
            warn!("expected to find header for height {}", firstuse.0);
            BlockHash::default()
        } else {
            *h[0].hash()
        }
    };

    let height = if firstuse.0 == MEMPOOL_HEIGHT {
        0
    } else {
        firstuse.0
    };

    Ok(json!({
        "block_hash": blockhash.to_hex(),
        "height": height,
        "block_height": height, // deprecated
        "tx_hash": firstuse.1.to_hex()
    }))
}
pub fn get_history(
    query: &Query,
    scripthash: &FullHash,
    timeout: &TimeoutTrigger,
) -> Result<Value> {
    let status = query.status(&scripthash, timeout)?;
    Ok(json!(Value::Array(
        status
            .history()
            .into_iter()
            .map(|item| json!({"height": item.0, "tx_hash": item.1.to_hex()}))
            .collect()
    )))
}

pub fn listunspent(
    query: &Query,
    scripthash: &FullHash,
    timeout: &TimeoutTrigger,
) -> Result<Value> {
    Ok(unspent_from_status(&query.status(scripthash, timeout)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ConfirmationState;
    use bitcoin::hash_types::Txid;
    use bitcoin_hashes::hex::FromHex;
    use serde_json::from_str;

    #[derive(Serialize, Deserialize)]
    struct Unspent {
        height: u32,
        tx_pos: u32,
        tx_hash: String,
        value: u64,
    }

    fn create_out(height: u32, txn_id: Txid) -> FundingOutput {
        FundingOutput {
            txn_id: txn_id,
            height: height,
            output_index: 0,
            value: 2020,
            state: ConfirmationState::InMempool,
        }
    }

    #[test]
    fn test_output_to_json_mempool() {
        // Mempool height is 0 in the json API
        let out = create_out(MEMPOOL_HEIGHT, Txid::default());
        let res: Unspent = from_str(&unspent_to_json(&out).to_string()).unwrap();
        assert_eq!(0, res.height);

        // Confirmed at block 5000
        let out = create_out(5000, Txid::default());
        let res: Unspent = from_str(&unspent_to_json(&out).to_string()).unwrap();
        assert_eq!(5000, res.height);
    }

    #[test]
    fn test_output_to_json_txid() {
        let hex = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeffffffffffffffffffffffffffffffff";
        let out = create_out(1, Txid::from_hex(hex).unwrap());
        let res: Unspent = from_str(&unspent_to_json(&out).to_string()).unwrap();
        assert_eq!(hex, res.tx_hash);
    }
}
