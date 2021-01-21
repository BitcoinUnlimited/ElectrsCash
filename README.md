# ElectrsCash - Electrum Server in Rust


[![license](https://img.shields.io/github/license/BitcoinUnlimited/ElectrsCash.svg)](https://github.com/BitcoinUnlimited/ElectrsCash/blob/master/LICENSE)
![CI](https://github.com/BitcoinUnlimited/ElectrsCash/workflows/Rust/badge.svg?branch=master&event=push)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat-square)](http://makeapullrequest.com)

An efficient implementation of Electrum Server.

ElectrsCash is an efficient implementation of Electrum Server and can be used
as a drop-in replacement for ElectrumX. In addition to the TCP RPC interface,
it also provides WebSocket support.

ElectrsCash fully implements the
[v1.4.3 Electrum Cash protocol](https://bitcoincash.network/electrum/)
and in addition to [useful extensions](doc/rpc.md), including CashAccounts.

The server indexes the entire Bitcoin Cash blockchain, and the resulting index
enables fast queries for blockchain applications and any given user wallet,
allowing the user to keep real-time track of his balances and his transaction
history.

When run on the user's own machine, there is no need for the wallet to
communicate with external Electrum servers,
thus preserving the privacy of the user's addresses and balances.

## Features

- Supports Electrum protocol [v1.4.3](https://bitcoincash.network/electrum/)
- Maintains an index over transaction inputs and outputs, allowing fast balance
  queries.
- Fast synchronization of the Bitcoin Cash blockchain on modest hardware
- Low index storage overhead (~20%), relying on a local full node for
  transaction retrieval.
- `txindex` is not required for the Bitcoin node, however it does improve
  performance.
- Uses a single [RocksDB](https://github.com/spacejam/rust-rocksdb) database
  for better consistency and crash recovery.

- Notable features unique to ElectrsCash

- Has [really good integration with Bitcoin Unlimited](https://github.com/BitcoinUnlimited/BitcoinUnlimited/blob/release/doc/bu-electrum-integration.md).
- Supports all Bitcoin Cash full nodes that havfe basic `bitcoind` RPC support.
- The best integration test coverage of all electrum server implementations.
  (see Tests section)
- [CashAccount support](https://honest.cash/v2/dagur/fast-cashaccount-lookups-using-bitbox-and-electrum-4781)

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

Bitcoin Unlimited builds ElectrsCash and runs the above tests as part of their
continuous integration.

## Linters

Code linting and formatting is enforced in the projects continuous integration.
When contributing, please run `cargo clippy` to catch common mistakes and
improve your code, as well as `cargo fmt` to format the code.
