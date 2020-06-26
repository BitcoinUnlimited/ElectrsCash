# ElectrsCash - Electrum Server in Rust


[![license](https://img.shields.io/github/license/BitcoinUnlimited/ElectrsCash.svg)](https://github.com/BitcoinUnlimited/ElectrsCash/blob/master/LICENSE)
![CI](https://github.com/BitcoinUnlimited/ElectrsCash/workflows/Rust/badge.svg?branch=master&event=push)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat-square)](http://makeapullrequest.com)

An efficient implementation of Electrum Server.

The motivation behind this project is to improve the Bitcoin Cash infrastructure
for lightweight clients, providing them with efficient backend services.

ElectrsCash extends the original Electrum protocol, supporting additional
technology well established in the ecosystem such as CashAccounts.

The server indexes the entire Bitcoin Cash blockchain, and the resulting index enables fast queries for any given user wallet,
allowing the user to keep real-time track of his balances and his transaction history using the [Electron Cash wallet](https://electroncash.org/).
Since it runs on the user's own machine, there is no need for the wallet to communicate with external Electrum servers,
thus preserving the privacy of the user's addresses and balances.

## Features

 * Supports Electrum protocol [v1.4.2](https://bitcoincash.network/electrum/)
 * Maintains an index over transaction inputs and outputs, allowing fast balance queries
 * Fast synchronization of the Bitcoin Cash blockchain on modest hardware
 * Low index storage overhead (~20%), relying on a local full node for transaction retrieval
 * `txindex` is not required for the Bitcoin node, however it does improve performance
 * Uses a single [RocksDB](https://github.com/spacejam/rust-rocksdb) database, for better consistency and crash recovery

## Notable features unique to ElectrsCash

 * [CashAccount support](https://honest.cash/v2/dagur/fast-cashaccount-lookups-using-bitbox-and-electrum-4781)
 * Supports major Bitcoin Cash full nodes, in addition to [full integration with Bitcoin Unlimited](https://github.com/BitcoinUnlimited/BitcoinUnlimited/blob/release/doc/bu-electrum-integration.md)
 * Deterministic builds
 * We're the only electrum server with good integration tests coverage (see Tests below)

## Usage

See [here](doc/usage.md) for installation, build and usage instructions.

## Index database

The database schema is described [here](doc/schema.md).

## Tests

Run unit tests with `cargo test`.

Integration tests are included in the [Bitcoin Unlimited test set](https://github.com/BitcoinUnlimited/BitcoinUnlimited/tree/dev/qa/rpc-tests). Look for tests prefixed with `electrum_`.

To run the tests, you need to:
- [Clone and build Bitcoin Unlimited](https://github.com/BitcoinUnlimited/BitcoinUnlimited/blob/release/doc/build-unix.md).
- run `./contrib/run_functional_tests.sh`.

## Linters

Code linting and formatting is enforced in the projects continuous integration. When contributing, please run `cargo clippy` to catch common mistakes and improve your code, as well as `cargo fmt` to format the code.
