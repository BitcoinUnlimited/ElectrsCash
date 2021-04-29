extern crate electrscash;

extern crate error_chain;
#[macro_use]
extern crate log;

use error_chain::ChainedError;
use std::process;
use std::sync::Arc;

use electrscash::{
    app::App,
    bulk,
    cache::{BlockTxIDsCache, TransactionCache},
    config::Config,
    daemon::Daemon,
    doslimit::{ConnectionLimits, GlobalLimits},
    errors::*,
    index::Index,
    metrics::Metrics,
    query::Query,
    rpc::Rpc,
    signal::Waiter,
    store::{full_compaction, is_compatible_version, is_fully_compacted, DbStore},
};

fn run_server(config: &Config) -> Result<()> {
    let signal = Waiter::start();
    let metrics = Arc::new(Metrics::new(config.monitoring_addr));
    metrics.start();
    let blocktxids_cache = Arc::new(BlockTxIDsCache::new(
        config.blocktxids_cache_size as u64,
        &*metrics,
    ));

    let daemon = Arc::new(Daemon::new(
        &config.daemon_dir,
        &config.blocks_dir,
        config.daemon_rpc_addr,
        config.cookie_getter(),
        config.network_type,
        signal.clone(),
        blocktxids_cache,
        &*metrics,
    )?);
    // Perform initial indexing.
    let compatible = {
        let store = DbStore::open(&config.db_path, config.low_memory, &*metrics);
        is_compatible_version(&store)
    };

    if !compatible {
        info!("Incompatible database. Running full reindex.");
        DbStore::destroy(&config.db_path);
    }
    let store = DbStore::open(&config.db_path, config.low_memory, &*metrics);
    let index = Index::load(
        &store,
        &daemon,
        &*metrics,
        config.index_batch_size,
        config.cashaccount_activation_height,
    )?;
    let store = if is_fully_compacted(&store) {
        store // initial import and full compaction are over
    } else if config.jsonrpc_import {
        // slower: uses JSONRPC for fetching blocks
        index.reload(&store); // load headers
        index.update(&store, &signal)?;
        full_compaction(store)
    } else {
        // faster, but uses more memory
        let store = bulk::index_blk_files(
            &daemon,
            config.bulk_index_threads,
            &&*metrics,
            &signal,
            store,
            config.cashaccount_activation_height,
        )?;
        let store = full_compaction(store);
        index.reload(&store); // make sure the block header index is up-to-date
        store
    }
    .enable_compaction(); // enable auto compactions before starting incremental index updates.

    let app = App::new(store, index, daemon, &config)?;
    let tx_cache = TransactionCache::new(config.tx_cache_size as u64, &*metrics);
    let query = Query::new(app.clone(), &*metrics, tx_cache, config.network_type)?;
    let relayfee = query.get_relayfee()?;
    let connection_limits = ConnectionLimits::new(
        config.rpc_timeout,
        config.scripthash_subscription_limit,
        config.scripthash_alias_bytes_limit,
    );
    let global_limits = Arc::new(GlobalLimits::new(
        config.rpc_max_connections,
        config.rpc_max_connections_shared_prefix,
        &*metrics,
    ));

    let mut server: Option<Rpc> = None; // Electrum RPC server

    let rpc_addr = config.electrum_rpc_addr;
    let ws_addr = config.electrum_ws_addr;
    electrscash::util::spawn_thread("ws", move || {
        electrscash::wstcp::start_ws_proxy(ws_addr, rpc_addr)
    });

    loop {
        let (headers_changed, new_tip) = app.update(&signal)?;
        let txs_changed = query.update_mempool()?;

        server = match server {
            Some(rpc) => {
                rpc.notify_scripthash_subscriptions(&headers_changed, txs_changed);
                if let Some(header) = new_tip {
                    rpc.notify_subscriptions_chaintip(header);
                }
                Some(rpc)
            }
            None => Some(Rpc::start(
                config.electrum_rpc_addr,
                query.clone(),
                metrics.clone(),
                relayfee,
                connection_limits,
                global_limits.clone(),
                config.rpc_buffer_size,
            )),
        };
        if let Err(err) = signal.wait(config.wait_duration) {
            info!("stopping server: {}", err);
            break;
        }
    }
    Ok(())
}

fn main() {
    let config = Config::from_args();
    if let Err(e) = run_server(&config) {
        error!("server failed: {}", e.display_chain());
        process::exit(1);
    }
}
