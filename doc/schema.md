# Index Schema

The index is stored at a single RocksDB database using the following schema:

## Transaction outputs' index

Allows efficiently finding all funding transactions for a specific address:

|  Code  | Script Hash Prefix   | Funding TxID Prefix   | Funding Output Index | Funding amount |   |
| ------ | -------------------- | --------------------- | -------------------- | -------------- | - |
| `b'O'` | `SHA256(script)[:8]` | `txid[:8]`            | `varint`             | `varint`       |   |

## Transaction inputs' index

Allows efficiently finding spending transaction of a specific output:

|  Code  | Funding TxID Prefix  | Funding Output Index  | Spending TxID Prefix  |   |
| ------ | -------------------- | --------------------- | --------------------- | - |
| `b'I'` | `txid[:8]`           | `varint`              | `txid[:8]`            |   |


## Full Transaction IDs

In order to save storage space, we store the full transaction IDs once, and use their 8-byte prefixes for the indexes above.

|  Code  | Transaction ID    |   | Confirmed height   |
| ------ | ----------------- | - | ------------------ |
| `b'T'` | `txid` (32 bytes) |   | `uint32`           |

Note that this mapping allows us to use `getrawtransaction` RPC to retrieve actual transaction data from without `-txindex` enabled
(by explicitly specifying the [blockhash](https://github.com/bitcoin/bitcoin/commit/497d0e014cc79d46531d570e74e4aeae72db602d)).

## CashAccount index

Allows finding all transactions containing CashAccount registration by name and block height.

|  Code  | Account name              | Registration TxID Prefix   |   |
| ------ | ------------------------- | -------------------------- | - |
| `b'C'` | `SHA256(name#height)[:8]` | `txid[:8]`                 |   |
