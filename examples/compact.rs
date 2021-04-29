/// Benchmark full compaction.
extern crate electrscash;

#[macro_use]
extern crate log;

extern crate error_chain;

use electrscash::{config::Config, errors::*, metrics::Metrics, store::DbStore};

use error_chain::ChainedError;

fn run(config: Config) -> Result<()> {
    if !config.db_path.exists() {
        panic!(
            "DB {:?} must exist when running this benchmark!",
            config.db_path
        );
    }

    let metrics = Metrics::new(config.monitoring_addr);
    metrics.start();

    let store = DbStore::open(&config.db_path, /*low_memory=*/ true, &metrics);
    store.compact();
    Ok(())
}

fn main() {
    if let Err(e) = run(Config::from_args()) {
        error!("{}", e.display_chain());
    }
}
