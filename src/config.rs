use bitcoincash::network::constants::Network;
use dirs_next::home_dir;
use std::convert::TryInto;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::CookieGetter;
use crate::errors::*;

// by default, serve on all IPv4 interfaces
const DEFAULT_BIND_ADDRESS: [u8; 4] = [0, 0, 0, 0];
const MONITOR_BIND_ADDRESS: [u8; 4] = [127, 0, 0, 1];
const DEFAULT_SERVER_ADDRESS: [u8; 4] = [127, 0, 0, 1];

mod internal {
    #![allow(unused)]
    #![allow(clippy::all)]

    include!(concat!(env!("OUT_DIR"), "/configure_me_config.rs"));
}

/// A simple error type representing invalid UTF-8 input.
pub struct InvalidUtf8(OsString);

impl fmt::Display for InvalidUtf8 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?} isn't a valid UTF-8 sequence", self.0)
    }
}

/// An error that might happen when resolving an address
pub enum AddressError {
    ResolvError { addr: String, err: std::io::Error },
    NoAddrError(String),
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AddressError::ResolvError { addr, err } => {
                write!(f, "Failed to resolve address {}: {}", addr, err)
            }
            AddressError::NoAddrError(addr) => write!(f, "No address found for {}", addr),
        }
    }
}

/// Newtype for an address that is parsed as `String`
///
/// The main point of this newtype is to provide better description than what `String` type
/// provides.
#[derive(Deserialize)]
pub struct ResolvAddr(String);

impl ::configure_me::parse_arg::ParseArg for ResolvAddr {
    type Error = InvalidUtf8;

    fn parse_arg(arg: &OsStr) -> std::result::Result<Self, Self::Error> {
        Self::parse_owned_arg(arg.to_owned())
    }

    fn parse_owned_arg(arg: OsString) -> std::result::Result<Self, Self::Error> {
        arg.into_string().map_err(InvalidUtf8).map(ResolvAddr)
    }

    fn describe_type<W: fmt::Write>(mut writer: W) -> fmt::Result {
        write!(writer, "a network address (will be resolved if needed)")
    }
}

impl ResolvAddr {
    /// Resolves the address.
    fn resolve(self) -> std::result::Result<SocketAddr, AddressError> {
        match self.0.to_socket_addrs() {
            Ok(mut iter) => iter.next().ok_or(AddressError::NoAddrError(self.0)),
            Err(err) => Err(AddressError::ResolvError { addr: self.0, err }),
        }
    }

    /// Resolves the address, but prints error and exits in case of failure.
    fn resolve_or_exit(self) -> SocketAddr {
        self.resolve().unwrap_or_else(|err| {
            eprintln!("Error: {}", err);
            std::process::exit(1)
        })
    }
}

/// This newtype implements `ParseArg` for `Network`.
#[derive(Deserialize)]
pub struct BitcoinNetwork(Network);

impl Default for BitcoinNetwork {
    fn default() -> Self {
        BitcoinNetwork(Network::Bitcoin)
    }
}

impl FromStr for BitcoinNetwork {
    type Err = <Network as FromStr>::Err;

    fn from_str(string: &str) -> std::result::Result<Self, Self::Err> {
        Network::from_str(string).map(BitcoinNetwork)
    }
}

impl ::configure_me::parse_arg::ParseArgFromStr for BitcoinNetwork {
    fn describe_type<W: fmt::Write>(mut writer: W) -> std::fmt::Result {
        write!(writer, "either 'bitcoin', 'testnet' or 'regtest'")
    }
}

impl From<BitcoinNetwork> for Network {
    fn from(s: BitcoinNetwork) -> Network {
        s.0
    }
}

/// Parsed and post-processed configuration
pub struct Config {
    // See below for the documentation of each field:
    pub log: stderrlog::StdErrLog,
    pub network_type: Network,
    pub db_path: PathBuf,
    pub daemon_dir: PathBuf,
    pub blocks_dir: PathBuf,
    pub daemon_rpc_addr: SocketAddr,
    pub electrum_rpc_addr: SocketAddr,
    pub electrum_ws_addr: SocketAddr,
    pub monitoring_addr: SocketAddr,
    pub jsonrpc_import: bool,
    pub wait_duration: Duration,
    pub index_batch_size: usize,
    pub bulk_index_threads: usize,
    pub tx_cache_size: usize,
    pub server_banner: String,
    pub blocktxids_cache_size: usize,
    pub cookie_getter: Arc<dyn CookieGetter>,
    pub rpc_timeout: u16,
    pub low_memory: bool,
    pub cashaccount_activation_height: u32,
    pub rpc_buffer_size: usize,
    pub scripthash_subscription_limit: u32,
    pub scripthash_alias_bytes_limit: u32,
    pub rpc_max_connections: u32,
    pub rpc_max_connections_shared_prefix: u32,
}

/// Returns default daemon directory
fn default_daemon_dir() -> PathBuf {
    let mut home = home_dir().unwrap_or_else(|| {
        eprintln!("Error: unknown home directory");
        std::process::exit(1)
    });
    home.push(".bitcoin");
    home
}

fn default_blocks_dir(daemon_dir: &Path) -> PathBuf {
    daemon_dir.join("blocks")
}

fn create_cookie_getter(
    cookie: Option<String>,
    cookie_file: Option<PathBuf>,
    daemon_dir: &Path,
) -> Arc<dyn CookieGetter> {
    match (cookie, cookie_file) {
        (None, None) => Arc::new(CookieFile::from_daemon_dir(daemon_dir)),
        (None, Some(file)) => Arc::new(CookieFile::from_file(file)),
        (Some(cookie), None) => Arc::new(StaticCookie::from_string(cookie)),
        (Some(_), Some(_)) => {
            eprintln!("Error: ambigous configuration - cookie and cookie_file can't be specified at the same time");
            std::process::exit(1);
        }
    }
}

/// Processes deprecation of cookie in favor of auth
fn select_auth(auth: Option<String>, cookie: Option<String>) -> Option<String> {
    match (cookie, auth) {
        (None, None) => None,
        (Some(value), None) => {
            eprintln!("WARNING: cookie option is deprecated and will be removed in the future!");
            eprintln!();
            eprintln!("You most likely want to use cookie_file instead.");
            eprintln!("If you really don't want to use cookie_file for a good reason and knowing the consequences use the auth option");
            eprintln!(
                "See authentication section in electrs usage documentation for more details."
            );
            eprintln!("https://github.com/romanz/electrs/blob/master/doc/usage.md#configuration-files-and-priorities");
            Some(value)
        }
        (None, Some(value)) => Some(value),
        (Some(_), Some(_)) => {
            eprintln!("Error: cookie and auth can't be specified at the same time");
            eprintln!("It looks like you made a mistake during migrating cookie option, please check your config.");
            std::process::exit(1);
        }
    }
}

impl Config {
    /// Parses args, env vars, config files and post-processes them
    pub fn from_args() -> Config {
        use internal::ResultExt;

        let system_config: &OsStr = "/etc/electrscash/config.toml".as_ref();
        let home_config = home_dir().map(|mut dir| {
            dir.extend(&[".electrscash", "config.toml"]);
            dir
        });
        let cwd_config: &OsStr = "electrscash.toml".as_ref();
        let configs = std::iter::once(cwd_config)
            .chain(home_config.as_ref().map(AsRef::as_ref))
            .chain(std::iter::once(system_config));

        let (mut config, _) =
            internal::Config::including_optional_config_files(configs).unwrap_or_exit();

        let db_subdir = match config.network {
            // We must keep the name "mainnet" due to backwards compatibility
            Network::Bitcoin => "mainnet",
            Network::Testnet => "testnet",
            Network::Regtest => "regtest",
            Network::Testnet4 => "testnet4",
            Network::Scalenet => "scalenet",
        };

        config.db_dir.push(db_subdir);

        let default_daemon_port = match config.network {
            Network::Bitcoin => 8332,
            Network::Testnet => 18332,
            Network::Regtest => 18443,
            Network::Testnet4 => 28332,
            Network::Scalenet => 38332,
        };
        let default_electrum_port = match config.network {
            Network::Bitcoin => 50001,
            Network::Testnet => 60001,
            Network::Regtest => 60401,
            Network::Testnet4 => 62001,
            Network::Scalenet => 63001,
        };
        let default_monitoring_port = match config.network {
            Network::Bitcoin => 4224,
            Network::Testnet => 14224,
            Network::Regtest => 24224,
            Network::Testnet4 => 34224,
            Network::Scalenet => 44224,
        };

        let default_ws_port = match config.network {
            Network::Bitcoin => 50003,
            Network::Testnet => 60003,
            Network::Regtest => 60403,
            Network::Testnet4 => 62003,
            Network::Scalenet => 63003,
        };

        let daemon_rpc_addr: SocketAddr = config.daemon_rpc_addr.map_or(
            (DEFAULT_SERVER_ADDRESS, default_daemon_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        let electrum_rpc_addr: SocketAddr = config.electrum_rpc_addr.map_or(
            (DEFAULT_BIND_ADDRESS, default_electrum_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        let electrum_ws_addr: SocketAddr = config.electrum_ws_addr.map_or(
            (DEFAULT_BIND_ADDRESS, default_ws_port).into(),
            ResolvAddr::resolve_or_exit,
        );
        let monitoring_addr: SocketAddr = config.monitoring_addr.map_or(
            (MONITOR_BIND_ADDRESS, default_monitoring_port).into(),
            ResolvAddr::resolve_or_exit,
        );

        match config.network {
            Network::Bitcoin => (),
            Network::Testnet => config.daemon_dir.push("testnet3"),
            Network::Regtest => config.daemon_dir.push("regtest"),
            Network::Testnet4 => config.daemon_dir.push("testnet4"),
            Network::Scalenet => config.daemon_dir.push("scalenet"),
        }

        let daemon_dir = &config.daemon_dir;
        let blocks_dir = config
            .blocks_dir
            .unwrap_or_else(|| default_blocks_dir(daemon_dir));

        let auth = select_auth(config.auth, config.cookie);
        let cookie_getter = create_cookie_getter(auth, config.cookie_file, daemon_dir);

        let mut log = stderrlog::new();
        log.verbosity(
            config
                .verbose
                .try_into()
                .expect("Overflow: Running electrs on less than 32 bit devices is unsupported"),
        );
        log.timestamp(if config.timestamp {
            stderrlog::Timestamp::Millisecond
        } else {
            stderrlog::Timestamp::Off
        });
        log.init().unwrap_or_else(|err| {
            eprintln!("Error: logging initialization failed: {}", err);
            std::process::exit(1)
        });
        // Could have been default, but it's useful to allow the user to specify 0 when overriding
        // configs.
        if config.bulk_index_threads == 0 {
            config.bulk_index_threads = num_cpus::get();
        }
        const MB: f32 = (1 << 20) as f32;
        let config = Config {
            log,
            network_type: config.network,
            db_path: config.db_dir,
            daemon_dir: config.daemon_dir,
            blocks_dir,
            daemon_rpc_addr,
            electrum_rpc_addr,
            electrum_ws_addr,
            monitoring_addr,
            jsonrpc_import: config.jsonrpc_import,
            wait_duration: Duration::from_secs(config.wait_duration_secs),
            index_batch_size: config.index_batch_size,
            bulk_index_threads: config.bulk_index_threads,
            tx_cache_size: (config.tx_cache_size_mb * MB) as usize,
            blocktxids_cache_size: (config.blocktxids_cache_size_mb * MB) as usize,
            server_banner: config.server_banner,
            cookie_getter,
            rpc_timeout: config.rpc_timeout as u16,
            low_memory: config.low_memory,
            cashaccount_activation_height: config.cashaccount_activation_height as u32,
            rpc_buffer_size: config.rpc_buffer_size,
            scripthash_subscription_limit: config.scripthash_subscription_limit,
            scripthash_alias_bytes_limit: config.scripthash_alias_bytes_limit,
            rpc_max_connections: config.rpc_max_connections,
            rpc_max_connections_shared_prefix: config.rpc_max_connections_shared_prefix,
        };
        eprintln!("{:?}", config);
        config
    }

    pub fn cookie_getter(&self) -> Arc<dyn CookieGetter> {
        Arc::clone(&self.cookie_getter)
    }
}

// CookieGetter + Debug isn't implemented in Rust, so we have to skip cookie_getter
macro_rules! debug_struct {
    ($name:ty, $($field:ident,)*) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    $(
                        .field(stringify!($field), &self.$field)
                    )*
                    .finish()
            }
        }
    }
}

debug_struct! { Config,
    log,
    network_type,
    db_path,
    daemon_dir,
    blocks_dir,
    daemon_rpc_addr,
    electrum_rpc_addr,
    electrum_ws_addr,
    monitoring_addr,
    jsonrpc_import,
    index_batch_size,
    bulk_index_threads,
    tx_cache_size,
    server_banner,
    blocktxids_cache_size,
    rpc_timeout,
    low_memory,
    cashaccount_activation_height,
    rpc_buffer_size,
    scripthash_subscription_limit,
    scripthash_alias_bytes_limit,
    rpc_max_connections,
    rpc_max_connections_shared_prefix,
}

struct StaticCookie {
    value: Vec<u8>,
}

impl StaticCookie {
    fn from_string(value: String) -> Self {
        StaticCookie {
            value: value.into(),
        }
    }
}

impl CookieGetter for StaticCookie {
    fn get(&self) -> Result<Vec<u8>> {
        Ok(self.value.clone())
    }
}

struct CookieFile {
    cookie_file: PathBuf,
}

impl CookieFile {
    fn from_daemon_dir(daemon_dir: &Path) -> Self {
        CookieFile {
            cookie_file: daemon_dir.join(".cookie"),
        }
    }

    fn from_file(cookie_file: PathBuf) -> Self {
        CookieFile { cookie_file }
    }
}

impl CookieGetter for CookieFile {
    fn get(&self) -> Result<Vec<u8>> {
        let contents = fs::read(&self.cookie_file).chain_err(|| {
            ErrorKind::Connection(format!(
                "failed to read cookie from {}",
                self.cookie_file.display()
            ))
        })?;
        Ok(contents)
    }
}
