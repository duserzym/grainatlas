//! One evictable-image budget for all tabs of the desktop window.
use std::sync::TryLockError;

use super::{AppState, Ordering, Session};
use grainatlas_core::project::session::DEFAULT_HEAVY_BUDGET;
use tauri::{AppHandle, Emitter};
use grainatlas_core::Ctl;
use super::Progress;

impl AppState {
    /// Run an allocating operation, then enforce the window budget even on error.
    /// Return owned results so the caller's session lock is released before trimming.
    pub(super) fn with_session<T>(&self, tab: u32, f: impl FnOnce(&mut Session) -> T) -> T {
        self.with_session_budget(tab, DEFAULT_HEAVY_BUDGET, f)
    }

    /// Called only from a blocking command. Reset cancellation after acquiring the
    /// session, so a queued command cannot clear the running command's cancel flag.
    pub(super) fn with_session_ctl<T>(&self, app: &AppHandle, tab: u32, f: impl FnOnce(&mut Session, &Ctl) -> T) -> T {
        let t = self.tab(tab);
        self.with_session(tab, |s| {
            t.cancel.store(false, Ordering::SeqCst);
            let cancel = || t.cancel.load(Ordering::SeqCst);
            let progress = |stage: &str, fraction: f32| {
                let _ = app.emit("grainatlas://progress", Progress { tab, stage: stage.to_string(), fraction });
            };
            f(s, &Ctl { cancel: &cancel, progress: &progress })
        })
    }

    fn with_session_budget<T>(&self, tab: u32, budget: usize, f: impl FnOnce(&mut Session) -> T) -> T {
        let t = self.tab(tab);
        let result = {
            let mut s = t.lock();
            let result = f(&mut s);
            t.evictable.store(s.evictable_bytes(), Ordering::Relaxed);
            result
        };
        self.trim_memory(tab, budget);
        result
    }

    fn trim_memory(&self, current: u32, budget: usize) {
        // No session lock is held while waiting for another eviction pass.
        // Serializing passes ensures concurrent completions cannot both skip
        // each other and leave idle tabs over budget.
        let _pass = self.memory_trim.lock().unwrap_or_else(|p| p.into_inner());
        let tabs: Vec<_> = self.tabs.lock().unwrap_or_else(|p| p.into_inner())
            .iter().map(|(&id, t)| (id, t.clone())).collect();
        let mut used = 0usize;
        let mut idle = Vec::new();
        for (id, t) in &tabs {
            // Never wait for a running command, including one in the caller's tab.
            let s = match t.session.try_lock() {
                Ok(s) => s,
                Err(TryLockError::Poisoned(p)) => p.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    used = used.saturating_add(t.evictable.load(Ordering::Relaxed));
                    continue;
                }
            };
            let bytes = s.evictable_bytes();
            t.evictable.store(bytes, Ordering::Relaxed);
            used = used.saturating_add(bytes);
            idle.push((*id, t, s, bytes));
        }
        idle.sort_by_key(|(id, _, s, _)| (s.last_heavy_use(), *id));
        for (id, t, mut s, before) in idle {
            if used <= budget {
                break;
            }
            if id == current {
                continue;
            }
            s.evict_to(before.saturating_sub(used - budget));
            let after = s.evictable_bytes();
            t.evictable.store(after, Ordering::Relaxed);
            used -= before - after;
        }
        // Busy tabs and an oversized current tab can temporarily exceed the
        // budget. Allocating commands publish their count and retry on completion.
    }
}

