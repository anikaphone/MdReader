use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub struct FileWatcher {
    _watcher: Option<RecommendedWatcher>,
    rx: Receiver<PathBuf>,
    current_watched: Option<PathBuf>,
    last_event_time: Instant,
}

impl FileWatcher {
    pub fn new() -> (Self, Sender<PathBuf>) {
        let (tx, rx) = channel();
        (
            Self {
                _watcher: None,
                rx,
                current_watched: None,
                last_event_time: Instant::now(),
            },
            tx,
        )
    }

    pub fn watch(&mut self, path: &Path) {
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => path.to_path_buf(),
        };

        if self.current_watched.as_ref() == Some(&path) {
            return;
        }

        self.current_watched = Some(path.clone());
        let (tx, rx) = channel();
        self.rx = rx;

        let target_path = path.clone();
        let watcher_res = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_)
                            if event.paths.iter().any(|p| {
                                p == &target_path || p.file_name() == target_path.file_name()
                            }) =>
                        {
                            let _ = tx.send(target_path.clone());
                        }
                        _ => {}
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        );

        if let Ok(mut watcher) = watcher_res {
            // Watch parent dir or the file itself
            let watch_target = path.parent().unwrap_or(&path);
            let _ = watcher.watch(watch_target, RecursiveMode::NonRecursive);
            self._watcher = Some(watcher);
        }
    }

    pub fn unwatch(&mut self) {
        self._watcher = None;
        self.current_watched = None;
    }

    /// Check if a reload is needed with 200ms debounce
    pub fn check_reload(&mut self) -> bool {
        let mut triggered = false;
        while self.rx.try_recv().is_ok() {
            triggered = true;
        }

        if triggered {
            let now = Instant::now();
            if now.duration_since(self.last_event_time) > Duration::from_millis(200) {
                self.last_event_time = now;
                return true;
            }
        }
        false
    }
}
