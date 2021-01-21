# Release notes

## 3.0.0 (21 January 2021)

### Relase notes

This version completes implementation of
[Protocol version 1.4.3](https://bitcoincash.network/electrum/). In addition
a custom call [`blockchain.transcation.get_confirmed_blockhash`](doc/rpc.md)
has been added to utilize the transaction index. This can replace bitcoind
txindex for those who want to optimize disk space.

WebSocket support has been added. As part of testing it, it has been used to
power the
[flipstarter overview site](https://flipstarters.bitcoincash.network/) and
[receipt generator](https://receipt.bitcoincash.network/) applications.

Support for Scalenet and Testnet4 has been added.

A bunch new metrics have been added, including [rpc
connections](https://bit.ly/3o5ADf4), [memory usage](https://bit.ly/2LI29m1)
and [cache churn](https://bit.ly/35ZGLiK) + more. See
[stats.bitcoincash.network](https://stats.bitcoincash.network) for demo with
all metrics.

The `blockchain.transcation.get` got a full overhaul and test coverage. It now
supports all fields as bitcoind. It is **more** consistent than bitcoind, as
every implementation (BU, BCHN, ...) has its quirks and in addition remove/add
fields depending on various transaction properties, such as if it's coinbase,
unconfirmed etc. ElectrsCash never removes or adds a field.

This is a major version release, which implies breaking changes. Please see
below.

### Changes

* [bug] Fix compatibility with Bitcoin ABC (#93)
* [bug] Fix issue "fee must be in decreasing order" (#98)
* [dos] Add RPC connection limit (#113)
* [dos] Limit connections by IP prefix (#114)
* [dos] Add scripthash subscription limit (#107)
* [maintainance] Bump crate dependencies and replace abandoned crates (#118)
* [maintainance] Remove batch fetching of unconfirmed transactions (#117)
* [maintainance] Remove use of `mut self` in rpc blockchain (#122)
* [maintainance] Replace use of `bitcoin_hashes` crate with `bitcoincash` (#97)
* [maintainance] Split out responsibility of Query (#116)
* [maintainance] Use prometheus imports (#100)
* [metrics] Add metric `electrscash_process_jemalloc_allocated` (#104)
* [metrics] Add metric `electrscash_process_jemalloc_resident` (#104)
* [metrics] Add metric `electrscash_rockdb_mem_readers_total` (#103)
* [metrics] Add metric `electrscash_rockdb_mem_table_total` (#103)
* [metrics] Add metric `electrscash_rockdb_mem_table_unflushed` (#103)
* [metrics] Add metric `electrscash_rpc_connections_rejeced_global` (#125)
* [metrics] Add metric `electrscash_rpc_connections_rejeced_prefix` (#125)
* [metrics] Add metric `electrscash_rpc_connections_total` (#125)
* [metrics] Add metric `electrscash_rpc_connections` (#125)
* [misc] Add WebSocket support (#109)
* [misc] Add testnet4 and scalenet support (#130)
* [misc] Remove --txid-limit (#106)
* [misc] Support more argument datatypes in python cli (#95)
* [misc] Update confiration defaults (#124)
* [misc] Use random eviction cache (#101)
* [performance] Change semantics of index-batch-size (#121)
* [performance] Store statushashes as FullHash (#105)
* [performance] Use generators with `load_txns_by_prefix` (#126)
* [performance] Use parallel iter for finding inputs/outputs (#127)
* [rpc] Add RPC `blockchain.address.get_mempool` (#120)
* [rpc] Add RPC `blockchain.address.subscribe` and `blockchain.address.unsubscribe`. (#108)
* [rpc] Add RPC `blockchain.scripthash.get_mempool` (#120)
* [rpc] Add RPC `blockchain.transaction.get_confirmed_blockhash` (#96)
* [rpc] Improve likeness with bitcoind getrawtransaction (#119, #128, #129)
* [rpc] Make height optional in `transaction.get_merkle` (#111)

### Breaking changes

#### Default listening interface

The default listening interfaces for the RPC has been changed from localhost to
all interfaces. The WebSocket interface also listens on all interfaces.

This projects goal is to be a high performant public Electrum server. Unlike
this projects predecessor, which aims to be a private server on your local
machine.

#### Parameter `--txid-limit` removed

The `--txid-limit` DoS parameter is removed. Please use `--rpc-timeout`
parameter instead for more accurate DoS limit.

For backward compatibility with existing configurations, this argument still
exists but now does nothing. It will be completely removed in next major
version of ElectrsCash.

#### Changes in `blockchain.transaction.get` verbose output

**tldr;** The `value` field in the `vout` entries of verbose transaction output is
replaced with `value_satoshis` and `value_coins`. It used to be in satoshis.

The `value` field will be reintroduced in later version of ElectrsCash
in unit coins rather than satoshis (satoshis / 100 000 000).

**Details:** The verbose output of `blockchain.transaction.get` is implemented
in ElectrsCash, rather than forwarded to the bitcoind node as stated in the
specification. This is a massive performance increase, as the transaction is
often in the local cache.

It was discovered that the `value` of `vout` entries were in satoshis, compared
to bitcoind where it is in coins (satoshis / 100 000 000). While satoshis is
more consistent with the rest of the electrum specification, it is incorrect
as the specification for this RPC call is to output "whatever bitcoind outputs".

Rather than simply changing the unit of the field at the risk of software that
we don't know of using this field silently failing, it has been temporarily
removed to allow such software to properly fail and corrected. The `value`
field will be re-added in a later version of ElectrsCash.

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
