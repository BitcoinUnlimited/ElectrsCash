extern crate electrscash;

extern crate error_chain;
#[macro_use]
extern crate log;

use error_chain::ChainedError;
use std::process;
use std::sync::Arc;
use std::time::Duration;

use electrscash::{
    app::App,
    bulk,
    cache::{BlockTxIDsCache, TransactionCache},
    config::Config,
    daemon::Daemon,
    errors::*,
    index::Index,
    metrics::Metrics,
    query::Query,
    rpc::RPC,
    signal::Waiter,
    store::{full_compaction, is_fully_compacted, DBStore},
};

fn run_server(config: &Config) -> Result<()> {
    let signal = Waiter::start();
    let metrics = Metrics::new(config.monitoring_addr);
    metrics.start();
    let blocktxids_cache = Arc::new(BlockTxIDsCache::new(config.blocktxids_cache_size, &metrics));

    let daemon = Daemon::new(
        &config.daemon_dir,
        config.daemon_rpc_addr,
        config.cookie_getter(),
        config.network_type,
        signal.clone(),
        blocktxids_cache,
        &metrics,
    )?;
    // Perform initial indexing from local blk*.dat block files.
    let store = DBStore::open(&config.db_path, /*low_memory=*/ config.jsonrpc_import);
    let index = Index::load(&store, &daemon, &metrics, config.index_batch_size)?;
    let store = if is_fully_compacted(&store) {
        store // initial import and full compaction are over
    } else if config.jsonrpc_import {
        index.update(&store, &signal)?; // slower: uses JSONRPC for fetching blocks
        full_compaction(store)
    } else {
        // faster, but uses more memory
        let store =
            bulk::index_blk_files(&daemon, config.bulk_index_threads, &metrics, &signal, store)?;
        let store = full_compaction(store);
        index.reload(&store); // make sure the block header index is up-to-date
        store
    }
    .enable_compaction(); // enable auto compactions before starting incremental index updates.

    let app = App::new(store, index, daemon, &config)?;
    let tx_cache = TransactionCache::new(config.tx_cache_size, &metrics);
    let query = Query::new(app.clone(), &metrics, tx_cache, config.txid_limit);

    let mut server = None; // Electrum RPC server

    loop {
        let (headers_changed, new_tip) = app.update(&signal)?;
        let txs_changed = query.update_mempool()?;
        let rpc = server
            .get_or_insert_with(|| RPC::start(config.electrum_rpc_addr, query.clone(), &metrics));
        rpc.notify_scripthash_subscriptions(&headers_changed, txs_changed);
        if let Some(header) = new_tip {
            rpc.notify_subscriptions_chaintip(header);
        }
        if let Err(err) = signal.wait(Duration::from_secs(5)) {
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
