use bitcoin::blockdata::opcodes;
use bitcoin::blockdata::script::{Builder, Script};
use bitcoincash_addr::{Address, HashType};
use crypto::digest::Digest;
use crypto::sha2::Sha256;
use std::convert::TryInto;

use crate::errors::*;

const HASH_LEN: usize = 32;
pub type FullHash = [u8; HASH_LEN];

pub trait ToLEHex {
    fn to_le_hex(&self) -> String;
}

impl ToLEHex for FullHash {
    fn to_le_hex(&self) -> String {
        let mut h = *self;
        h.reverse();
        hex::encode(h)
    }
}

pub fn full_hash(hash: &[u8]) -> FullHash {
    hash.try_into().expect("failed to convert into FullHash")
}

pub fn addr_to_scripthash(addr: &str) -> Result<FullHash> {
    let decoded = match Address::decode(addr) {
        Ok(d) => d,
        Err(e) => return Err(format!("{:?}", e).into()),
    };

    let pubkey: Script = match decoded.hash_type {
        HashType::Key => Builder::new()
            .push_opcode(opcodes::all::OP_DUP)
            .push_opcode(opcodes::all::OP_HASH160)
            .push_slice(&decoded.body[..])
            .push_opcode(opcodes::all::OP_EQUALVERIFY)
            .push_opcode(opcodes::all::OP_CHECKSIG)
            .into_script(),
        HashType::Script => Builder::new()
            .push_opcode(opcodes::all::OP_HASH160)
            .push_slice(&decoded.body[..])
            .push_opcode(opcodes::all::OP_EQUAL)
            .into_script(),
    };
    Ok(compute_script_hash(pubkey.as_bytes()))
}

pub fn compute_script_hash(data: &[u8]) -> FullHash {
    let mut hash = FullHash::default();
    let mut sha2 = Sha256::new();
    sha2.input(data);
    sha2.result(&mut hash);
    hash
}

pub fn decode_scripthash(hexstr: &str) -> Result<FullHash> {
    let mut bytes = hex::decode(hexstr).chain_err(|| "failed to parse scripthash")?;
    if bytes.len() != 32 {
        return Err(format!("invalid scripthash length ({} != 32)", bytes.len()).into());
    }
    bytes.reverse();
    Ok(full_hash(&bytes[..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_scripthash() {
        assert_eq!(
            &hex::decode("3318537dfb3135df9f3d950dbdf8a7ae68dd7c7dfef61ed17963ff80f3850474")
                .unwrap()[..],
            decode_scripthash("740485f380ff6379d11ef6fe7d7cdd68aea7f8bd0d953d9fdf3531fb7d531833")
                .unwrap()
        );

        // too short
        assert!(decode_scripthash("740485").is_err());
    }

    #[test]
    fn test_addr_to_scripthash_p2pkh() {
        // Protocol specification test vector
        let scripthash =
            decode_scripthash("8b01df4e368ea28f8dc0423bcf7a4923e3a12d307c875e47a0cfbf90b5c39161")
                .unwrap();
        assert_eq!(
            scripthash,
            addr_to_scripthash("bitcoincash:qp3wjpa3tjlj042z2wv7hahsldgwhwy0rq9sywjpyy").unwrap()
        );

        assert_eq!(
            scripthash,
            addr_to_scripthash("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").unwrap()
        );
    }

    #[test]
    fn test_addr_to_scripthash_p2sh() {
        // eatbch
        let scripthash =
            decode_scripthash("829ce9ce75a8a8a01bf27a7365655506614ef0b8f5a7ecbef19093951a73b686")
                .unwrap();
        assert_eq!(
            scripthash,
            addr_to_scripthash("bitcoincash:pp8skudq3x5hzw8ew7vzsw8tn4k8wxsqsv0lt0mf3g").unwrap()
        );
        assert_eq!(
            scripthash,
            addr_to_scripthash("38ty1qB68gHsiyZ8k3RPeCJ1wYQPrUCPPr").unwrap()
        );
    }

    #[test]
    fn test_addr_to_scripthash_garbage() {
        assert!(addr_to_scripthash("garbage").is_err());
    }

    #[test]
    fn test_to_le_hex() {
        let hex = "829ce9ce75a8a8a01bf27a7365655506614ef0b8f5a7ecbef19093951a73b686";
        let scripthash: FullHash = decode_scripthash(hex).unwrap();
        assert_eq!(hex, scripthash.to_le_hex());
    }
}
