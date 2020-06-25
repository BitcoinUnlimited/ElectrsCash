use crate::def::{
    ELECTRSCASH_VERSION, PROTOCOL_HASH_FUNCTION, PROTOCOL_VERSION_MAX, PROTOCOL_VERSION_MIN,
};
use crate::errors::*;
use crate::query::Query;
use crate::rpc::parseutil::{rpc_arg_error, str_from_value};
use bitcoin_hashes::hex::ToHex;
use serde_json::Value;
use std::sync::Arc;

use version_compare::Version;

/**
 * This file contains implementations and tests of RPC calls starting with
 * server.*
 */

// The default argument to server.version
const SPEC_DEFAULT_VERSION: &str = "1.4";

fn best_match(client_min: &Version, client_max: &Version) -> String {
    let our_min = Version::from(PROTOCOL_VERSION_MIN).unwrap();
    let our_max = Version::from(PROTOCOL_VERSION_MAX).unwrap();

    if *client_max >= our_max {
        return our_max.as_str().into();
    }

    if *client_max <= our_min {
        return our_min.as_str().into();
    }

    if *client_min >= our_min && *client_max <= our_max {
        return client_max.as_str().into();
    }

    our_min.as_str().into()
}

fn best_match_response(client_min: &Version, client_max: &Version) -> Value {
    json!([versionstr(), best_match(client_min, client_max)])
}

fn versionstr() -> String {
    format!("ElectrsCash {}", ELECTRSCASH_VERSION)
}

pub fn parse_version(version: &str) -> Result<Version> {
    let version = Version::from(&version).chain_err(|| rpc_arg_error("invalid version string"))?;
    Ok(version)
}

pub fn server_version(params: &[Value]) -> Result<Value> {
    // default to spec default on missing argument
    let default_version = json!(SPEC_DEFAULT_VERSION);
    let val = params.get(1).unwrap_or(&default_version);

    if let Ok(versionstr) = str_from_value(Some(val), "version") {
        let version = parse_version(&versionstr)?;
        return Ok(best_match_response(&version, &version));
    }

    if let Some(minmax_list) = val.as_array() {
        let min = str_from_value(Some(&minmax_list[0]), "version")?;
        let min = parse_version(&min)?;
        let max = str_from_value(Some(&minmax_list[1]), "version")?;
        let max = parse_version(&max)?;
        return Ok(best_match_response(&min, &max));
    }

    Err(rpc_arg_error("invalid value in version argument").into())
}

pub fn server_banner(query: &Arc<Query>) -> Result<Value> {
    Ok(json!(query.get_banner()?))
}

pub fn server_donation_address() -> Result<Value> {
    Ok(Value::Null)
}

pub fn server_peers_subscribe() -> Result<Value> {
    Ok(json!([]))
}

pub fn server_features(query: &Arc<Query>) -> Result<Value> {
    let genesis_header = query.get_headers(&[0])[0].clone();
    Ok(json!({
        "genesis_hash" : genesis_header.hash().to_hex(),
        "hash_function": PROTOCOL_HASH_FUNCTION,
        "protocol_max": PROTOCOL_VERSION_MAX,
        "protocol_min": PROTOCOL_VERSION_MIN,
        "server_version": versionstr(),
        "firstuse": ["1.0"]
    }))
}

pub fn server_add_peer() -> Result<Value> {
    Ok(json!(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_version_noarg() {
        let resp = server_version(&[]).unwrap();
        let resp = resp.as_array().unwrap();

        assert!(resp[0].is_string());
        assert_eq!(resp[1].as_str().unwrap(), SPEC_DEFAULT_VERSION);
    }

    #[test]
    fn test_server_version_strarg() {
        let clientver = json!("bestclient 1.0");
        let resp = server_version(&[clientver.clone(), json!("1.3")]).unwrap();
        assert_eq!(resp[1].as_str().unwrap(), PROTOCOL_VERSION_MIN);
        let resp = server_version(&[clientver.clone(), json!("13.3.7")]).unwrap();
        assert_eq!(resp[1].as_str().unwrap(), PROTOCOL_VERSION_MAX);
    }

    #[test]
    fn test_server_version_minmax() {
        let clientver = json!("bestclient 1.0");
        // client max is higher than our max, we should return our max
        let resp = server_version(&[clientver.clone(), json!(["1.4", "13.3.7"])]).unwrap();
        assert_eq!(resp[1].as_str().unwrap(), PROTOCOL_VERSION_MAX);

        // client max is lower than our min, we shoud return our min
        let resp = server_version(&[clientver.clone(), json!(["1.2", "1.3"])]).unwrap();
        assert_eq!(resp[1].as_str().unwrap(), PROTOCOL_VERSION_MIN);

        // client max is somewhere between our max and min, return same version
        let client_max = "1.4.1";
        let resp = server_version(&[clientver.clone(), json!([PROTOCOL_VERSION_MIN, client_max])])
            .unwrap();
        assert_eq!(resp[1].as_str().unwrap(), client_max);
    }
}
