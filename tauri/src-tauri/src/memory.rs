//! One evictable-image budget for all tabs of the desktop window.
use std::sync::TryLockError;

use super::{AppState, Ordering, Session};
use grainatlas_core::project::session::DEFAULT_HEAVY_BUDGET;

impl AppState {
    /// Run an allocating operation, then enforce the window budget even on error.
    /// Return owned results so the caller's session lock is released before trimming.
    pub(super) fn with_session<T>(&self, tab: u32, f: impl FnOnce(&mut Session) -> T) -> T {
        self.with_session_budget(tab, DEFAULT_HEAVY_BUDGET, f)
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

#[cfg(test)]
mod tests {
    use super::*;
    use grainatlas_core::{Ctl, source::DirSource};
    use std::{path::PathBuf, sync::Arc};

    fn fixture() -> Session {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tm4000/Sweden_XS_150X");
        let mut s = Session::new("fixture");
        s.open_source(Arc::new(DirSource::new(root).unwrap()), "fixture", "now").unwrap();
        s.set_memory_budget(usize::MAX);
        let id = s.project.as_ref().unwrap().areas[0].id.clone();
        s.stitch(&id, "", &Ctl::NONE, &|| 0.0).unwrap();
        s
    }

    fn setup() -> (AppState, String, usize) {
        let state = AppState::default();
        for id in 0..3 {
            state.with_session(id, |s| *s = fixture());
        }
        let t = state.tab(0);
        let s = t.lock();
        let id = s.project.as_ref().unwrap().areas[0].id.clone();
        let bytes = s.evictable_bytes();
        assert!(bytes > 0);
        (state, id, bytes)
    }

    fn total(state: &AppState) -> usize {
        (0..3).map(|id| state.tab(id).lock().evictable_bytes()).sum()
    }

    #[test]
    fn window_budget_evicts_lru_tabs_and_protects_the_caller() {
        let (state, area, bytes) = setup();
        // Fixture setup touches 0, 1, 2. Cached heavy use in 0 makes 1 oldest.
        state.with_session_budget(0, 2 * bytes, |s| s.histogram(&area).unwrap());
        assert!(state.tab(1).lock().last_heavy_use() < state.tab(2).lock().last_heavy_use());
        assert_eq!(total(&state), 2 * bytes);
        assert_eq!(state.tab(1).lock().evictable_bytes(), 0);
        assert_eq!(state.tab(2).lock().evictable_bytes(), bytes);
        // Caller stays resident even when it alone exceeds the window budget.
        state.trim_memory(0, bytes - 1);
        assert_eq!(total(&state), bytes);
        assert_eq!(state.tab(0).lock().evictable_bytes(), bytes);
        assert_eq!(state.tab(0).lock().memory_report().budget, usize::MAX);
        // A failing operation still enforces the window budget.
        let result = state.with_session_budget(1, bytes, |s| {
            s.histogram(&area).unwrap();
            Err::<(), _>("after allocation")
        });
        assert!(result.is_err());
        assert_eq!(total(&state), bytes);
        assert_eq!(state.tab(0).lock().evictable_bytes(), 0);
        assert_eq!(state.tab(1).lock().evictable_bytes(), bytes);
    }

    #[test]
    fn concurrent_completions_leave_idle_tabs_within_budget() {
        let (state, area, bytes) = setup();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for id in 0..2 {
                let (state, area, barrier) = (&state, &area, &barrier);
                scope.spawn(move || state.with_session_budget(id, bytes, |s| {
                    s.histogram(area).unwrap();
                    barrier.wait();
                }));
            }
        });
        assert_eq!(total(&state), bytes);
        assert_eq!(state.tab(2).lock().evictable_bytes(), 0);
    }

    #[test]
    fn busy_tabs_are_counted_without_waiting_and_retry_on_completion() {
        let (state, area, bytes) = setup();
        let t = state.tab(0);
        let busy = t.lock(); // Oldest tab is busy; trimming must return without it.
        state.trim_memory(2, 2 * bytes);
        assert_eq!(state.tab(1).lock().evictable_bytes(), 0);
        assert_eq!(busy.evictable_bytes(), bytes);
        drop(busy);
        // The busy command's completion enforces again, protecting its own tab.
        state.with_session_budget(0, bytes, |s| s.histogram(&area).unwrap());
        assert_eq!(total(&state), bytes);
        assert_eq!(state.tab(2).lock().evictable_bytes(), 0);
    }
}
