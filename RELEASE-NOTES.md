# Release notes

## 2.0.0 (21 August 2020)
* [bug] Fix deadlock on Ctrl+C (#89)
* [bug] Update subscription statistics correctly (#89)
* [contrib] Add generic client for testing RPC methods (#83)
* [misc] Add a 'blocks-dir' option analogous to bitcoind's '-blocksdir' (#89)
* [misc] Add configuration `wait-duration-secs` to set custom main loop wait duration
* [misc] External notification of block update via SIGUSR1 (#78)
* [misc] Improve rpc timeout trigger (#81)
* [misc] Log progress while waiting for IBD (#76)
* [misc] Switch from `bitcoin` crate dependency to `bitcoincash` (#88)
* [rpc] Add `blockchain.address.get_balance` (#72)
* [rpc] Add `blockchain.address.get_first_use`
* [rpc] Add `blockchain.address.get_first_use` (#72)
* [rpc] Add `blockchain.address.get_history` (#72)
* [rpc] Add `blockchain.address.get_scripthash` (#85)
* [rpc] Add `blockchain.address.listunspent` (#72)
* [rpc] Add `blockchain.scripthash.unsubscribe` (#71)
* [rpc] Show fee for unconfirmed transactions (#89)
* [rpc] Update `blockchain.scripthash.get_first_use`

### Breaking changes

This release contains an RPC optimization that requires Bitcoin Unlimited v1.9
or BCHN v22.0.0 and will not work with older versions.

## 1.1.1 (10 April 2020)
* [bug] Fix protocol-negotiation in `server.version` response (#61)
* [bug] Fix dropped notification due to full client buffer (#60)
* [bug] Reduce log level on client errors (#64)

This is a bug-fix only release. No breaking changes.

## 1.1.0 (6 April 2020)
* [bug] Don't index cashaccounts before activation height
* [misc] Add database version and reindex on incompatible database.
* [misc] Allow loading config file from specified place via `--conf`
* [misc] Allow setting `--cookie-file` path via configuration
* [misc] Clean up RPC threads after connection is closed
* [misc] Setting `--network` now takes `bitcoin` instead of `mainnet` as parameter
* [performance] Better Cashaccount memory handling.
* [performance] Better client subscription change detection
* [performance] Better db indexes for faster scripthash lookups.
* [performance] Better transaction caching.
* [qa] Script for running integration tests.
* [rpc] Add RPC timeouts (DoS mitigation)
* [rpc] Bump protocol version to 1.4.1
* [rpc] Identify transactions with unconfirmed parents
* [rpc] Implement `blockchain.scripthash.get_first_use` RPC method.
* [rpc] Implement `server.features` RPC method.
* [rpc] Improved error messages with error codes.
* [rpc] Return error on unknown method rather than disconnecting the client.
* [rpc] Use bitcoind's relay fee rather than hardcoded.

Note: This release has database changes incompatible with previous versions.
At first startup after the upgrade, ElectrsCash will do a full reindex.

## 1.0.0 (18 September 2019)

* Cache capacity is now defined in megabytes, rather than number of entries.
* Support Rust >=1.34
* Use `configure_me` instead of `clap` to support config files, environment variables and man pages (@Kixunil)
* Revert LTO build (to fix deterministic build)
* Allow stopping bulk indexing via SIGINT/SIGTERM
* Cache list of transaction IDs for blocks
* Prefix Prometheus metrics with 'electrscash_'

## 0.7.0 (13 June 2019)

* Support Bitcoin Core 0.18
* Build with LTO
* Allow building with latest Rust (via feature flag)
* Use iterators instead of returning vectors (@Kixunil)
* Use atomics instead of `Mutex<u64>` (@Kixunil)
* Better handling invalid blocks (@azuchi)

## 0.6.2 (17 May 2019)

* Support Rust 1.32 (for Debian)

## 0.6.1 (9 May 2019)

* Fix crash during initial sync
* Switch to `signal-hook` crate

## 0.6.0 (29 Apr 2019)

* Update to Rust 1.34
* Prefix Prometheus metrics with 'electrs_'
* Update RocksDB crate to 0.12.1
* Update Bitcoin crate to 0.18
* Support latest bitcoind mempool entry vsize field name
* Fix "chain-trimming" reorgs
* Serve by default on IPv4 localhost

## 0.5.0 (3 Mar 2019)

* Limit query results, to prevent RPC server to get stuck (see `--txid-limit` flag)
* Update RocksDB crate to 0.11
* Update Bitcoin crate to 0.17

## 0.4.3 (23 Dec 2018)

* Support Rust 2018 edition (1.31)
* Upgrade to Electrum protocol 1.4 (from 1.2)
* Let server banner be configurable via command-line flag
* Improve query.get_merkle_proof() performance

## 0.4.2 (22 Nov 2018)

* Update to rust-bitcoin 0.15.1
* Use bounded LRU cache for transaction retrieval
* Support 'server.ping' and partially 'blockchain.block.header' Electrum RPC

## 0.4.1 (14 Oct 2018)

* Don't run full compaction after initial import is over (when using JSONRPC)

## 0.4.0 (22 Sep 2018)

* Optimize for low-memory systems by using different RocksDB settings
* Rename `--skip_bulk_import` flag to `--jsonrpc-import`

## 0.3.2 (14 Sep 2018)

* Optimize block headers processing during startup
* Handle TCP disconnections during long RPCs
* Use # of CPUs for bulk indexing threads
* Update rust-bitcoin to 0.14
* Optimize block headers processing during startup


## 0.3.1 (20 Aug 2018)

* Reconnect to bitcoind only on transient errors
* Poll mempool after transaction broadcasting

## 0.3.0 (14 Aug 2018)

* Optimize for low-memory systems
* Improve compaction performance
* Handle disconnections from bitcoind by retrying
* Make `blk*.dat` ingestion more robust
* Support regtest network
* Support more Electrum RPC methods
* Export more Prometheus metrics (CPU, RAM, file descriptors)
* Add `scripts/run.sh` for building and running `electrs`
* Add some Python tools (as API usage examples)
* Change default Prometheus monitoring ports

## 0.2.0 (14 Jul 2018)

* Allow specifying custom bitcoind data directory
* Allow specifying JSONRPC cookie from commandline
* Improve initial bulk indexing performance
* Support 32-bit systems

## 0.1.0 (2 Jul 2018)

* Announcement: https://lists.linuxfoundation.org/pipermail/bitcoin-dev/2018-July/016190.html
* Published to https://crates.io/electrs and https://docs.rs/electrs
