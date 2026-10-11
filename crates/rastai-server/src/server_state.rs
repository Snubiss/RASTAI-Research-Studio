//! `Server`, the plain struct every handler reads, shared as `Arc<Server>` (layout section 8.2). Tables sit behind
//! std locks that are never held across an `.await`.
//!
//! M1-26 part 1 has the fields startup and sign-in need. The registry (M1-27), the bus (M1-28), the node links and
//! identity (M1-42, M1-47), the plugin host, the gateway and the rest join with the tasks that build them.

use std::sync::RwLock;

use rastai_core::paths::DataPaths;
use rastai_core::settings::ResolvedSettings;
use rastai_store::store::Store;
use tokio::sync::watch;

use crate::http::sessions::AuthState;

pub struct Server {
    pub paths: DataPaths,
    pub settings: RwLock<ResolvedSettings>,
    pub store: Store,
    /// Login codes, sessions and the owner token's hash.
    pub auth: AuthState,
    /// Set to true to stop the daemon (`request_shutdown`): the signal handler and `system.shutdown` (M1-27) do.
    /// The listeners watch it.
    pub shutdown: watch::Sender<bool>,
}

pub fn build_server(paths: DataPaths, settings: ResolvedSettings, store: Store, auth: AuthState) -> Server {
    let (shutdown, _first_receiver) = watch::channel(false);
    Server {
        paths,
        settings: RwLock::new(settings),
        store,
        auth,
        shutdown,
    }
}
