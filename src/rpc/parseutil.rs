use crate::errors::*;
use crate::scripthash::decode_scripthash;
use crate::scripthash::FullHash;
use bitcoin_hashes::hex::FromHex;
use bitcoin_hashes::Hash;
use serde_json::Value;

pub fn bool_from_value(val: Option<&Value>, name: &str) -> Result<bool> {
    let val = val.chain_err(|| rpc_arg_error(&format!("missing {}", name)))?;
    let val = val
        .as_bool()
        .chain_err(|| rpc_arg_error(&format!("not a bool {}", name)))?;
    Ok(val)
}

pub fn bool_from_value_or(val: Option<&Value>, name: &str, default: bool) -> Result<bool> {
    if val.is_none() {
        return Ok(default);
    }
    bool_from_value(val, name)
}

pub fn hash_from_value<T: Hash>(val: Option<&Value>) -> Result<T> {
    let hash = val.chain_err(|| rpc_arg_error("missing hash"))?;
    let hash = hash
        .as_str()
        .chain_err(|| rpc_arg_error("non-string hash"))?;
    let hash = T::from_hex(hash).chain_err(|| rpc_arg_error("non-hex hash"))?;
    Ok(hash)
}

pub fn scripthash_from_value(val: Option<&Value>) -> Result<FullHash> {
    let script_hash = val.chain_err(|| rpc_arg_error("missing scripthash"))?;
    let script_hash = script_hash
        .as_str()
        .chain_err(|| rpc_arg_error("non-string scripthash"))?;
    let script_hash =
        decode_scripthash(script_hash).chain_err(|| rpc_arg_error("invalid scripthash"))?;
    Ok(script_hash)
}

pub fn rpc_arg_error(what: &str) -> ErrorKind {
    ErrorKind::RpcError(RpcErrorCode::InvalidParams, what.to_string())
}

pub fn str_from_value(val: Option<&Value>, name: &str) -> Result<String> {
    let string = val.chain_err(|| rpc_arg_error(&format!("missing {}", name)))?;
    let string = string
        .as_str()
        .chain_err(|| rpc_arg_error(&format!("{} is not a string", name)))?;
    Ok(string.into())
}

pub fn usize_from_value(val: Option<&Value>, name: &str) -> Result<usize> {
    let val = val.chain_err(|| rpc_arg_error(&format!("missing {}", name)))?;
    let val = val
        .as_u64()
        .chain_err(|| rpc_arg_error(&format!("non-integer {}", name)))?;
    Ok(val as usize)
}

pub fn usize_from_value_or(val: Option<&Value>, name: &str, default: usize) -> Result<usize> {
    if val.is_none() {
        return Ok(default);
    }
    usize_from_value(val, name)
}
