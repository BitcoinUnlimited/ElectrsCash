# ElectrsCash - Electrum Server in Rust

[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat-square)](http://makeapullrequest.com)
[![Build Status](https://travis-ci.org/BitcoinUnlimited/ElectrsCash.svg?branch=master)](https://travis-ci.org/BitcoinUnlimited/ElectrsCash)

An efficient implementation of Electrum Server, patch set on top of
[electrs](https://github.com/romanz/electrs).

The motivation behind this project is to improve the Bitcoin Cash infrastructure
for lightweight clients, providing them with efficient backend services.

ElectrsCash extends the original Electrum protocol, supporting additional
technology well established in the ecosystem such as CashAccounts.

The server indexes the entire Bitcoin Cash blockchain, and the resulting index enables fast queries for any given user wallet,
allowing the user to keep real-time track of his balances and his transaction history using the [Electron Cash wallet](https://electroncash.org/).
Since it runs on the user's own machine, there is no need for the wallet to communicate with external Electrum servers,
thus preserving the privacy of the user's addresses and balances.

## Features

 * Supports Electrum protocol [v1.4](https://electrumx.readthedocs.io/en/latest/protocol.html)
 * Maintains an index over transaction inputs and outputs, allowing fast balance queries
 * Fast synchronization of the Bitcoin Cash blockchain on modest hardware
 * Low index storage overhead (~20%), relying on a local full node for transaction retrieval
 * `txindex` is not required for the Bitcoin node, however it does improve performance
 * Uses a single [RocksDB](https://github.com/spacejam/rust-rocksdb) database, for better consistency and crash recovery

## Notable features unique to ElectrsCash

 * [CashAccount support](https://honest.cash/v2/dagur/fast-cashaccount-lookups-using-bitbox-and-electrum-4781)
 * Supports major Bitcoin Cash full nodes, in addition to [full integration with Bitcoin Unlimited](https://github.com/BitcoinUnlimited/BitcoinUnlimited/blob/release/doc/bu-electrum-integration.md)
 * Deterministic builds
 * [Integration tests with](doc/integration-tests.md) `bitcoind` in regtest

## Usage

See [here](doc/usage.md) for installation, build and usage instructions.

## Index database

The database schema is described [here](doc/schema.md).
