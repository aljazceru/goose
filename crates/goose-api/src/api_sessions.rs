use dashmap::DashMap;
use goose::agents::Agent;
use std::sync::{atomic::{AtomicU64, Ordering}, Arc, LazyLock};
use tokio::sync::Mutex;
use uuid::Uuid;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct ApiSession {
    pub agent: Arc<Mutex<Agent>>, // agent for this session
    last_active: AtomicU64,
}

impl ApiSession {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent)),
            last_active: AtomicU64::new(current_timestamp()),
        }
    }

    pub fn touch(&self) {
        self.last_active.store(current_timestamp(), Ordering::Relaxed);
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        current_timestamp() - self.last_active.load(Ordering::Relaxed) > ttl.as_secs()
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub static SESSIONS: LazyLock<DashMap<Uuid, ApiSession>> = LazyLock::new(DashMap::new);

pub const SESSION_TIMEOUT_SECS: u64 = 3600;

pub fn cleanup_expired_sessions() {
    let ttl = Duration::from_secs(SESSION_TIMEOUT_SECS);
    SESSIONS.retain(|_, sess| !sess.is_expired(ttl));
}

