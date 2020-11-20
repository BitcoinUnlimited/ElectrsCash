use rocksdb::perf::get_memory_usage_stats;
use std::path::{Path, PathBuf};
use std::str::from_utf8;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::def::DATABASE_VERSION;
use crate::metrics::Metrics;
use crate::util::spawn_thread;
use crate::util::Bytes;

#[derive(Clone)]
pub struct Row {
    pub key: Bytes,
    pub value: Bytes,
}

impl Row {
    pub fn into_pair(self) -> (Bytes, Bytes) {
        (self.key, self.value)
    }
}

pub trait ReadStore: Sync {
    fn get(&self, key: &[u8]) -> Option<Bytes>;
    fn scan(&self, prefix: &[u8]) -> Vec<Row>;
}

pub trait WriteStore: Sync {
    fn write<I: IntoIterator<Item = Row>>(&self, rows: I, sync: bool);
    fn flush(&self);
}

#[derive(Clone)]
struct Options {
    path: PathBuf,
    bulk_import: bool,
    low_memory: bool,
}

pub struct DBStore {
    db: Arc<rocksdb::DB>,
    opts: Options,
    stats_thread: Option<thread::JoinHandle<()>>,
    stats_thread_kill: Arc<(Mutex<bool>, Condvar)>,
}

impl DBStore {
    fn open_opts(opts: Options, metrics: &Metrics) -> Self {
        debug!("opening DB at {:?}", opts.path);
        let mut db_opts = rocksdb::Options::default();
        db_opts.create_if_missing(true);
        // db_opts.set_keep_log_file_num(10);
        db_opts.set_max_open_files(if opts.bulk_import { 16 } else { 256 });
        db_opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
        db_opts.set_compression_type(rocksdb::DBCompressionType::Snappy);
        db_opts.set_target_file_size_base(256 << 20);
        db_opts.set_write_buffer_size(256 << 20);
        db_opts.set_disable_auto_compactions(opts.bulk_import); // for initial bulk load
        db_opts.set_advise_random_on_open(!opts.bulk_import); // bulk load uses sequential I/O
        if !opts.low_memory {
            db_opts.set_compaction_readahead_size(1 << 20);
        }

        let is_new_db = !opts.path.exists();

        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_size(if opts.low_memory { 256 << 10 } else { 1 << 20 });
        #[allow(clippy::mutex_atomic)]
        let mut store = DBStore {
            db: Arc::new(rocksdb::DB::open(&db_opts, &opts.path).unwrap()),
            opts,
            stats_thread: None,
            stats_thread_kill: Arc::new((Mutex::new(false), Condvar::new())),
        };
        if is_new_db {
            store.write(vec![version_marker()], true);
            store.flush();
        }
        store.start_stats_thread(metrics);
        store
    }

    fn start_stats_thread(&mut self, metrics: &Metrics) {
        let mem_table_total = metrics.gauge_int(prometheus::Opts::new(
            format!("electrscash_rockdb_mem_table_total_{:p}", &self.db),
            "Rockdb approximate memory usage of all the mem-tables".to_string(),
        ));

        let mem_table_unflushed = metrics.gauge_int(prometheus::Opts::new(
            format!("electrscash_rockdb_mem_table_unflushed_{:p}", &self.db),
            "Rocksdb approximate usage of un-flushed mem-tables".to_string(),
        ));

        let mem_table_readers_total = metrics.gauge_int(prometheus::Opts::new(
            format!("electrscash_rockdb_mem_table_readers_total_{:p}", &self.db),
            "Rocksdb approximate memory usage of all the table readers".to_string(),
        ));

        let dbptr = Arc::clone(&self.db);
        let kill = Arc::clone(&self.stats_thread_kill);

        self.stats_thread = Some(spawn_thread("dbstats", move || {
            let (killthread, cvar) = &*kill;
            loop {
                let k = killthread.lock().unwrap();
                let result = cvar.wait_timeout(k, Duration::from_secs(5)).unwrap();
                if *result.0 {
                    // kill thread
                    mem_table_total.set(0);
                    mem_table_unflushed.set(0);
                    mem_table_readers_total.set(0);
                    return;
                }
                let mem_usage = get_memory_usage_stats(Some(&[&*dbptr]), None);

                if let Ok(usage) = mem_usage {
                    mem_table_total.set(usage.mem_table_total as i64);
                    mem_table_unflushed.set(usage.mem_table_unflushed as i64);
                    mem_table_readers_total.set(usage.mem_table_readers_total as i64)
                }
            }
        }));
    }

    /// Opens a new RocksDB at the specified location.
    pub fn open(path: &Path, low_memory: bool, metrics: &Metrics) -> Self {
        DBStore::open_opts(
            Options {
                path: path.to_path_buf(),
                bulk_import: true,
                low_memory,
            },
            metrics,
        )
    }

    pub fn enable_compaction(self) -> Self {
        let mut opts = self.opts.clone();
        if opts.bulk_import {
            opts.bulk_import = false;
            info!("enabling auto-compactions");
            let opts = [("disable_auto_compactions", "false")];
            self.db.set_options(&opts).unwrap();
        }
        self
    }

    pub fn compact(self) -> Self {
        info!("starting full compaction");
        self.db.compact_range(None::<&[u8]>, None::<&[u8]>); // would take a while
        info!("finished full compaction");
        self
    }

    pub fn iter_scan(&self, prefix: &[u8]) -> ScanIterator {
        ScanIterator {
            prefix: prefix.to_vec(),
            iter: self.db.prefix_iterator(prefix),
            done: false,
        }
    }

    pub fn destroy(path: &Path) {
        match rocksdb::DB::destroy(&rocksdb::Options::default(), path) {
            Ok(_) => debug!("Database destroyed"),
            Err(err) => info!("Clould not destory database: {}", err),
        }
    }
}

pub struct ScanIterator<'a> {
    prefix: Vec<u8>,
    iter: rocksdb::DBIterator<'a>,
    done: bool,
}

impl<'a> Iterator for ScanIterator<'a> {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        if self.done {
            return None;
        }
        let (key, value) = self.iter.next()?;
        if !key.starts_with(&self.prefix) {
            self.done = true;
            return None;
        }
        Some(Row {
            key: key.to_vec(),
            value: value.to_vec(),
        })
    }
}

impl ReadStore for DBStore {
    fn get(&self, key: &[u8]) -> Option<Bytes> {
        self.db.get(key).unwrap().map(|v| v.to_vec())
    }

    // TODO: use generators
    fn scan(&self, prefix: &[u8]) -> Vec<Row> {
        let mut rows = vec![];
        for (key, value) in self.db.iterator(rocksdb::IteratorMode::From(
            prefix,
            rocksdb::Direction::Forward,
        )) {
            if !key.starts_with(prefix) {
                break;
            }
            rows.push(Row {
                key: key.to_vec(),
                value: value.to_vec(),
            });
        }
        rows
    }
}

impl WriteStore for DBStore {
    fn write<I: IntoIterator<Item = Row>>(&self, rows: I, sync: bool) {
        let mut batch = rocksdb::WriteBatch::default();
        for row in rows {
            batch.put(row.key.as_slice(), row.value.as_slice());
        }
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(sync);
        opts.disable_wal(!sync);
        self.db.write_opt(batch, &opts).unwrap();
    }

    fn flush(&self) {
        let mut opts = rocksdb::WriteOptions::new();
        opts.set_sync(true);
        opts.disable_wal(false);
        let empty = rocksdb::WriteBatch::default();
        self.db.write_opt(empty, &opts).unwrap();
    }
}

impl Drop for DBStore {
    fn drop(&mut self) {
        trace!("closing DB at {:?}", self.opts.path);

        // Stop exporting memory stats. The thread holds a copy of the db instance, so we need to
        // wait for it to exit for db to close.
        let (flag, cvar) = &*self.stats_thread_kill;
        *flag.lock().unwrap() = true;
        cvar.notify_one();
        self.stats_thread.take().map(thread::JoinHandle::join);
        trace!("done closing db");
    }
}

fn full_compaction_marker() -> Row {
    Row {
        key: b"F".to_vec(),
        value: b"".to_vec(),
    }
}

pub fn version_marker() -> Row {
    Row {
        key: b"VER".to_vec(),
        value: DATABASE_VERSION.into(),
    }
}

pub fn is_compatible_version(store: &dyn ReadStore) -> bool {
    let version = store.get(&version_marker().key);
    match version {
        Some(v) => match from_utf8(&v) {
            Ok(v) => v == DATABASE_VERSION,
            Err(_) => false,
        },
        None => false,
    }
}

pub fn full_compaction(store: DBStore) -> DBStore {
    store.flush();
    let store = store.compact().enable_compaction();
    store.write(vec![full_compaction_marker()], true);
    store
}

pub fn is_fully_compacted(store: &dyn ReadStore) -> bool {
    let marker = store.get(&full_compaction_marker().key);
    marker.is_some()
}
