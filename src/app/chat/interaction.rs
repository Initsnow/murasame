use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

pub static IS_GENERATING: AtomicBool = AtomicBool::new(false);
pub static LAST_INTERACTION_TS: AtomicI64 = AtomicI64::new(0);

pub fn record_interaction() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    LAST_INTERACTION_TS.store(now, Ordering::Relaxed);
}

pub fn get_last_interaction_ts() -> i64 {
    LAST_INTERACTION_TS.load(Ordering::Relaxed)
}
