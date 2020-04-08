use crate::errors::*;
use serde_json::Value;

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
