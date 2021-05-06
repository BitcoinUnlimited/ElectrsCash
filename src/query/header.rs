use crate::app::App;
use crate::errors::*;
use crate::mempool::MEMPOOL_HEIGHT;
use crate::query::queryutil::txrow_by_txid;
use crate::util::HeaderEntry;
use bitcoincash::hash_types::Txid;
use std::sync::Arc;

pub struct HeaderQuery {
    app: Arc<App>,
}

impl HeaderQuery {
    pub fn new(app: Arc<App>) -> HeaderQuery {
        HeaderQuery { app }
    }

    /// Get header for the block that given transaction was confirmed in.
    /// Height is optional, but makes Lookup faster.
    pub fn get_by_txid(
        &self,
        txid: &Txid,
        blockheight: Option<u32>,
    ) -> Result<Option<HeaderEntry>> {
        // Lookup in confirmed transactions' index
        let height = match blockheight {
            Some(height) => {
                if height == MEMPOOL_HEIGHT {
                    return Ok(None);
                }
                height
            }
            None => {
                txrow_by_txid(self.app.read_store(), &txid)
                    .chain_err(|| format!("not indexed tx {}", txid))?
                    .height
            }
        };

        let header = self
            .app
            .index()
            .get_header(height as usize)
            .chain_err(|| format!("missing header at height {}", height))?;
        Ok(Some(header))
    }

    pub fn best(&self) -> Option<HeaderEntry> {
        self.app.index().best_header()
    }

    pub fn at_height(&self, height: usize) -> Option<HeaderEntry> {
        self.app.index().get_header(height)
    }

    /// Get the height of block where a transaction was confirmed, or None if it's
    /// not confirmed.
    /// TODO: Move to TxQuery
    pub fn get_confirmed_height_for_tx(&self, txid: &Txid) -> Option<u32> {
        match txrow_by_txid(self.app.read_store(), txid) {
            Some(txrow) => Some(txrow.height),
            None => None,
        }
    }
}
