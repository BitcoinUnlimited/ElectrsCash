use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::thread;
use std::time::Duration;

use prometheus::{
    self, Encoder, Gauge, GaugeVec, Histogram, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts,
};

use crate::errors::*;
use crate::util::spawn_thread;

pub struct Metrics {
    reg: prometheus::Registry,
    addr: SocketAddr,
}

impl Metrics {
    pub fn new(addr: SocketAddr) -> Metrics {
        Metrics {
            reg: prometheus::Registry::new(),
            addr,
        }
    }

    /// Constructor for use in unittests
    pub fn dummy() -> Metrics {
        Metrics {
            reg: prometheus::Registry::new(),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1234),
        }
    }

    pub fn counter_int(&self, opts: prometheus::Opts) -> IntCounter {
        let c = IntCounter::with_opts(opts).unwrap();
        self.reg.register(Box::new(c.clone())).unwrap();
        c
    }

    pub fn counter_int_vec(&self, opts: prometheus::Opts, labels: &[&str]) -> IntCounterVec {
        let c = IntCounterVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(c.clone())).unwrap();
        c
    }

    pub fn gauge_float(&self, opts: prometheus::Opts) -> Gauge {
        let g = Gauge::with_opts(opts).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn gauge_float_vec(&self, opts: prometheus::Opts, labels: &[&str]) -> GaugeVec {
        let g = GaugeVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn gauge_int_vec(&self, opts: prometheus::Opts, labels: &[&str]) -> IntGaugeVec {
        let g = IntGaugeVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn gauge_int(&self, opts: prometheus::Opts) -> IntGauge {
        let g = IntGauge::with_opts(opts).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn histogram(&self, opts: prometheus::HistogramOpts) -> Histogram {
        let h = Histogram::with_opts(opts).unwrap();
        self.reg.register(Box::new(h.clone())).unwrap();
        h
    }

    pub fn histogram_vec(&self, opts: prometheus::HistogramOpts, labels: &[&str]) -> HistogramVec {
        let h = HistogramVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(h.clone())).unwrap();
        h
    }

    pub fn start(&self) {
        let server = tiny_http::Server::http(self.addr).unwrap_or_else(|e| {
            panic!(
                "failed to start monitoring HTTP server at {}: {}",
                self.addr, e
            )
        });
        start_process_exporter(self);
        let reg = self.reg.clone();
        spawn_thread("metrics", move || loop {
            if let Err(e) = handle_request(&reg, server.recv()) {
                error!("http error: {}", e);
            }
        });
    }
}

fn handle_request(
    reg: &prometheus::Registry,
    request: io::Result<tiny_http::Request>,
) -> io::Result<()> {
    let request = request?;
    let mut buffer = vec![];
    prometheus::TextEncoder::new()
        .encode(&reg.gather(), &mut buffer)
        .unwrap();
    let response = tiny_http::Response::from_data(buffer);
    request.respond(response)
}

struct Stats {
    utime: f64,
    rss: u64,
    fds: usize,
}

fn parse_stats() -> Result<Stats> {
    let value =
        fs::read_to_string("/proc/self/stat").chain_err(|| "failed to read /proc/self/stat")?;
    let parts: Vec<&str> = value.split_whitespace().collect();
    let page_size = page_size::get() as u64;
    let ticks_per_second = sysconf::raw::sysconf(sysconf::raw::SysconfVariable::ScClkTck)
        .expect("failed to get _SC_CLK_TCK") as f64;

    let parse_part = |index: usize, name: &str| -> Result<u64> {
        parts
            .get(index)
            .chain_err(|| format!("missing {}: {:?}", name, parts))?
            .parse::<u64>()
            .chain_err(|| format!("invalid {}: {:?}", name, parts))
    };

    // For details, see '/proc/[pid]/stat' section at `man 5 proc`:
    let utime = parse_part(13, "utime")? as f64 / ticks_per_second;
    let rss = parse_part(23, "rss")? * page_size;
    let fds = fs::read_dir("/proc/self/fd")
        .chain_err(|| "failed to read /proc/self/fd directory")?
        .count();
    Ok(Stats { utime, rss, fds })
}

fn start_process_exporter(metrics: &Metrics) {
    let rss = metrics.gauge_int(Opts::new(
        "electrscash_process_memory_rss",
        "Resident memory size [bytes]",
    ));
    let cpu = metrics.gauge_float_vec(
        Opts::new(
            "electrscash_process_cpu_usage",
            "CPU usage by this process [seconds]",
        ),
        &["type"],
    );
    let fds = metrics.gauge_int(Opts::new(
        "electrscash_process_open_fds",
        "# of file descriptors",
    ));

    let jemalloc_allocated = metrics.gauge_int(Opts::new(
        "electrscash_process_jemalloc_allocated",
        "# of bytes allocated by the application.",
    ));

    let jemalloc_resident = metrics.gauge_int(Opts::new(
        "electrscash_process_jemalloc_resident",
        "# of bytes in physically resident data pages mapped by the allocator",
    ));

    spawn_thread("exporter", move || {
        let e = jemalloc_ctl::epoch::mib().unwrap();
        let allocated = jemalloc_ctl::stats::allocated::mib().unwrap();
        let resident = jemalloc_ctl::stats::resident::mib().unwrap();
        loop {
            if let Ok(stats) = parse_stats() {
                cpu.with_label_values(&["utime"]).set(stats.utime as f64);
                rss.set(stats.rss as i64);
                fds.set(stats.fds as i64);
            }
            e.advance().unwrap();
            jemalloc_allocated.set(allocated.read().unwrap() as i64);
            jemalloc_resident.set(resident.read().unwrap() as i64);

            thread::sleep(Duration::from_secs(5));
        }
    });
}
