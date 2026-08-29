//! Pause injection while the launched game window is unfocused.
//!
//! Wine games run under XWayland, so the EWMH properties on the X server
//! tell us which window is active and which process owns it. The game's
//! window almost never belongs to the direct child: wine spawns a process
//! tree and the window lands on a grandchild — so ownership is checked
//! against the whole descendant tree of the launched process, not one pid.
//!
//! A Wayland-native game has no X11 windows at all: the active window then
//! reads as the desktop, which would wrongly pause input during play. The
//! client list disambiguates — "desktop active" only means unfocused when
//! the game owns X11 windows (it lost focus); otherwise it is a Wayland
//! surface we cannot observe and input stays active, the same degrade as a
//! missing X server.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// WM_STATE's "not managed by the window manager" value: the window exists
/// but was never mapped (or was withdrawn). Vestigial XWayland windows of
/// Wayland-native games carry this — or no WM_STATE at all.
const WITHDRAWN_STATE: u32 = 0;

use x11rb::connection::Connection;
use x11rb::protocol::res::ConnectionExt as ResExt;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

/// How many transient-parent hops to walk looking for the game pid; wine
/// wraps game windows in a small chain.
const MAX_PARENT_HOPS: usize = 8;
/// How long the /proc descendant snapshot stays fresh. Wine spawns and
/// exits processes constantly; matching against a stale tree both misses
/// new windows and keeps matching dead pids.
const TREE_REFRESH: Duration = Duration::from_secs(2);

pub struct FocusWatcher {
    conn: RustConnection,
    root: Window,
    child_pid: u32,
    active_atom: u32,
    pid_atom: u32,
    transient_atom: u32,
    client_list_atom: u32,
    wm_state_atom: u32,
    /// Whether the X-Resource extension answered its version handshake.
    /// RetroArch and other raw X11 clients never set `_NET_WM_PID`; XRes is
    /// the only way to map their windows to a process.
    xres: bool,
    tree: std::cell::RefCell<TreeCache>,
}

struct TreeCache {
    at: Instant,
    pids: HashSet<u32>,
}

impl FocusWatcher {
    /// None when there is no X server to ask; callers treat that as
    /// always-focused.
    pub fn for_child(child_pid: u32) -> Option<Self> {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots.get(screen).map(|screen| screen.root)?;
        let atom = |name: &[u8]| -> Option<u32> {
            Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
        };
        let xres = conn
            .res_query_version(1, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some();
        Some(Self {
            active_atom: atom(b"_NET_ACTIVE_WINDOW")?,
            pid_atom: atom(b"_NET_WM_PID")?,
            transient_atom: atom(b"WM_TRANSIENT_FOR")?,
            client_list_atom: atom(b"_NET_CLIENT_LIST")?,
            wm_state_atom: atom(b"WM_STATE")?,
            xres,
            root,
            child_pid,
            tree: std::cell::RefCell::new(TreeCache {
                at: Instant::now() - TREE_REFRESH,
                pids: HashSet::new(),
            }),
            conn,
        })
    }

    /// Whether the game's window currently has input focus. Unknown states
    /// count as focused so a WM without these hints never pauses the game.
    pub fn game_is_focused(&self) -> bool {
        let game_pids = self.game_pids();
        let active = self.active_window();
        let Some(active) = active else {
            return true;
        };
        let client_windows = self.client_windows();
        // A Wayland-focused window never appears in _NET_ACTIVE_WINDOW: on
        // GNOME Wayland the property holds a stale XWayland id that is not a
        // managed client (or nothing at all). Such an active window carries
        // no focus information — same degrade as the desktop being active.
        let desktop_like =
            active == 0 || active == self.root || !client_windows.contains(&active);
        let active_in_tree = !desktop_like && self.window_pid_in_tree(active, &game_pids, 0);
        let game_has_windows = desktop_like
            && client_windows.iter().any(|window| {
                self.window_pid(*window).is_some_and(|pid| game_pids.contains(&pid))
                    && self.window_is_managed(*window)
            });
        is_focused(active_in_tree, desktop_like, game_has_windows)
    }

    /// Descendants of the launched process, refreshed from /proc at most
    /// once per [`TREE_REFRESH`]. Unknown states fall back to the direct
    /// child so a transient /proc hiccup cannot unmatch everything.
    fn game_pids(&self) -> HashSet<u32> {
        let mut cache = self.tree.borrow_mut();
        if cache.at.elapsed() >= TREE_REFRESH {
            cache.pids = descendant_set(self.child_pid, &proc_parent_pairs());
            cache.at = Instant::now();
        }
        if cache.pids.is_empty() {
            let mut fallback = HashSet::new();
            fallback.insert(self.child_pid);
            return fallback;
        }
        cache.pids.clone()
    }

    fn active_window(&self) -> Option<Window> {
        self.conn
            .get_property(
                false,
                self.root,
                self.active_atom,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?
            .value32()
            .and_then(|mut values| values.next())
    }

    /// Whether `window` — or an ancestor in its transient chain — is owned
    /// by a game process. The hop limit keeps a WM-supplied chain loop from
    /// spinning forever.
    fn window_pid_in_tree(&self, window: Window, pids: &HashSet<u32>, depth: usize) -> bool {
        if depth > MAX_PARENT_HOPS {
            return false;
        }
        if self.window_pid(window).is_some_and(|pid| pids.contains(&pid)) {
            return true;
        }
        match self.transient_parent(window) {
            Some(parent) if parent != 0 && parent != self.root => {
                self.window_pid_in_tree(parent, pids, depth + 1)
            }
            _ => false,
        }
    }

    /// The window manager's managed X11 clients.
    fn client_windows(&self) -> Vec<Window> {
        let Ok(cookie) = self
            .conn
            .get_property(false, self.root, self.client_list_atom, AtomEnum::WINDOW, 0, 1024)
        else {
            return Vec::new();
        };
        let Ok(reply) = cookie.reply() else {
            return Vec::new();
        };
        let Some(windows) = reply.value32() else {
            return Vec::new();
        };
        // Collect first: the iterator borrows the reply, and a tail
        // expression temporary would outlive it.
        windows.collect()
    }

    /// Whether the window manager manages this window: `WM_STATE` exists and
    /// is not Withdrawn. Wayland-native games keep vestigial XWayland
    /// windows that were never managed; counting those as the game's "X11
    /// presence" made a focused Wayland game read as alt-tabbed away.
    fn window_is_managed(&self, window: Window) -> bool {
        let state = self
            .conn
            .get_property(false, window, self.wm_state_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut values| values.next()));
        state.is_some_and(|state| state != WITHDRAWN_STATE)
    }

    fn window_pid(&self, window: Window) -> Option<u32> {
        if let Some(pid) = self
            .conn
            .get_property(false, window, self.pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()
            .and_then(|mut values| values.next())
        {
            return Some(pid);
        }
        self.xres_pid(window)
    }

    /// XRes fallback for windows without `_NET_WM_PID`: find the X client
    /// whose resource range contains the window, then ask for that
    /// client's process id. On XWayland this reports the real client
    /// process, which is how RetroArch's raw XCB windows become matchable.
    fn xres_pid(&self, window: Window) -> Option<u32> {
        if !self.xres {
            return None;
        }
        let clients = self
            .conn
            .res_query_clients()
            .ok()?
            .reply()
            .ok()?
            .clients;
        let owner = clients.into_iter().find(|client| {
            client_owns_window(window, client.resource_base, client.resource_mask)
        })?;
        let reply = self
            .conn
            .res_query_client_ids(&[ClientIdSpec {
                client: owner.resource_base,
                mask: ClientIdMask::LOCAL_CLIENT_PID,
            }])
            .ok()?
            .reply()
            .ok()?;
        reply.ids.first().and_then(|value| value.value.first().copied())
    }

    fn transient_parent(&self, window: Window) -> Option<Window> {
        let cookie = self
            .conn
            .get_property(false, window, self.transient_atom, AtomEnum::WINDOW, 0, 1)
            .ok()?;
        let reply = cookie.reply().ok()?;
        reply.value32().and_then(|mut values| values.next())
    }
}

/// Whether `window` falls inside an X client's allocated resource range.
/// Resource ids are `base | (index & mask)`, so masking the low bits off
/// must yield the base again.
fn client_owns_window(window: u32, base: u32, mask: u32) -> bool {
    mask != 0 && window & !mask == base
}

/// Focus decision given what the X server answered. "Desktop-like" covers
/// the desktop being active AND focus being unobservable (Wayland window
/// focused, where `_NET_ACTIVE_WINDOW` holds a stale non-client id).
/// "Desktop active" is then ambiguous: unfocused for an X11 game (it has
/// managed windows on the client list but none active), unobservable for a
/// Wayland-native one (no managed X11 windows, so input stays active rather
/// than wrongly pausing).
fn is_focused(active_in_game_tree: bool, desktop_active: bool, game_has_x11_windows: bool) -> bool {
    if active_in_game_tree {
        return true;
    }
    if !desktop_active {
        return false;
    }
    !game_has_x11_windows
}

/// All pids reachable from `root` through the parent links in `pairs`
/// (from /proc): the process itself plus every descendant.
fn descendant_set(root: u32, pairs: &[(u32, u32)]) -> HashSet<u32> {
    let mut children: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    for (pid, parent) in pairs {
        children.entry(*parent).or_default().push(*pid);
    }
    let mut found = HashSet::new();
    let mut queue = vec![root];
    while let Some(pid) = queue.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if found.insert(*child) {
                queue.push(*child);
            }
        }
    }
    found
}

/// (pid, parent pid) for every process exposing it in /proc. Entries whose
/// stat disappears mid-walk are simply skipped; errors mean an empty list,
/// which degrades to matching only the direct child.
fn proc_parent_pairs() -> Vec<(u32, u32)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
            if let Some(parent) = parse_stat_ppid(&stat) {
                pairs.push((pid, parent));
            }
        }
    }
    pairs
}

/// The ppid field of /proc/<pid>/stat. The comm field may contain spaces
/// and parentheses, so everything before the final ')' is skipped.
fn parse_stat_ppid(stat: &str) -> Option<u32> {
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
    // After the ')' the kernel lists state (field 3), ppid (field 4), ...
    fields.next()?;
    fields.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{client_owns_window, descendant_set, is_focused, parse_stat_ppid};

    #[test]
    fn test_client_owns_window_matches_resource_ranges() {
        // A client with base 0x04000000 and a 22-bit mask owns its range.
        let (base, mask) = (0x0400_0000, 0x003F_FFFF);
        assert!(client_owns_window(0x0400_0001, base, mask));
        assert!(client_owns_window(base + mask, base, mask));
        assert!(!client_owns_window(base + mask + 1, base, mask));
        assert!(!client_owns_window(0x0440_0000, base, mask));
        assert!(!client_owns_window(0x0600_0001, base, mask));
        // A zero mask owns nothing (and would mask every window to zero).
        assert!(!client_owns_window(0x0400_0001, base, 0));
    }

    #[test]
    fn test_descendant_set_walks_the_whole_tree() {
        let pairs = [(1, 0), (10, 1), (11, 1), (20, 10), (30, 2)];
        let found = descendant_set(1, &pairs);
        assert!(found.contains(&10) && found.contains(&11) && found.contains(&20));
        assert!(!found.contains(&1)); // the root itself is not a descendant
        assert!(!found.contains(&30));
        // A parent-less root still terminates.
        assert!(descendant_set(99, &pairs).is_empty());
    }

    #[test]
    fn test_parse_stat_ppid_skips_comm_with_parentheses() {
        assert_eq!(parse_stat_ppid("123 (wine64-preloader) S 45 46 46"), Some(45));
        assert_eq!(
            parse_stat_ppid("124 (weird (name) here) R 7 8 8"),
            Some(7)
        );
        assert_eq!(parse_stat_ppid("125 (x) Z"), None);
        assert_eq!(parse_stat_ppid("garbage"), None);
    }

    #[test]
    fn test_focus_decision_matches_window_states() {
        // Game window active: focused, whatever else is true.
        assert!(is_focused(true, false, true));
        // Another X11 app active: unfocused.
        assert!(!is_focused(false, false, true));
        // Desktop active with an X11 game on the client list: it lost focus.
        assert!(!is_focused(false, true, true));
        // Desktop active with a Wayland-native game: unobservable, stay live.
        assert!(is_focused(false, true, false));
    }
}
