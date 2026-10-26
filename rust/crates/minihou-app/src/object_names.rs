//! `object_names_v1`: fuzzy search of Blender object names for the Blender Object node's `object` field.
//!
//! Only while the field is being edited, the app sends `object_names_query` {request_id, query, limit}
//! (debounced [`DEBOUNCE`] s; one on focus). The add-on scans `bpy.data.objects` time-sliced on its main thread
//! (a few ms per timer tick) and answers `object_names` {request_id, query, hash, count, partial, scanned, ms,
//! matches:[{name, type, mesh}]} with at most [`LIMIT`] ranked matches; it may send partial answers first.
//! The full list never crosses the link. Responses whose request id is not the newest are discarded; answers
//! are cached by (scene hash, query). Without the capability (older add-on) the field is a plain text box.
//!
//! The link threads and the UI meet in [`HUB`] (short mutex sections only; nothing here blocks the UI thread).
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

pub const CAPABILITY: &str = "object_names_v1";
pub const QUERY: &str = "object_names_query";
pub const RESPONSE: &str = "object_names";
/// Seconds between the last keystroke and the request.
pub const DEBOUNCE: f64 = 0.12;
/// A query Blender has not answered after this long is abandoned, so the field stops waiting.
pub const TIMEOUT: f64 = 3.0;
/// Most matches one response carries (and the add-on computes).
pub const LIMIT: usize = 50;
const MAX_NAME_BYTES: usize = 256;
const CACHE_ENTRIES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub name: String,
    pub kind: String,
    pub mesh: bool,
}

fn word_start(chars: &[char], j: usize) -> bool {
    if j == 0 { return true; }
    let (a, b) = (chars[j - 1], chars[j]);
    " _-./:".contains(a) || (a.is_lowercase() && b.is_uppercase()) || (a.is_alphabetic() && b.is_ascii_digit())
}

/// Fuzzy score of `name` for a lower-cased, non-empty `query` (None: no match). Same rules as the add-on's
/// `object_names.score`: exact > prefix > substring (word-start bonus) > subsequence (contiguity and
/// word-start bonuses, gap penalty); shorter names win ties.
pub fn score(query: &str, name: &str) -> Option<i32> {
    if query.is_empty() { return Some(0); }
    let low = name.to_lowercase();
    let len = low.chars().count() as i32;
    if low == query { return Some(1000); }
    if low.starts_with(query) { return Some(900 - len); }
    let chars: Vec<char> = name.chars().collect();
    let lows: Vec<char> = low.chars().collect();
    if chars.len() != lows.len() { return None; } // case mapping changed the length: no fuzzy positions
    if let Some(byte) = low.find(query) {
        let i = low[..byte].chars().count();
        return Some(700 - i as i32 - len / 4 + if word_start(&chars, i) { 50 } else { 0 });
    }
    let (mut pos, mut s, mut prev) = (0usize, 0i32, usize::MAX - 1);
    for c in query.chars() {
        let j = (pos..lows.len()).find(|&k| lows[k] == c)?;
        if j == prev.wrapping_add(1) { s += 10; }
        if word_start(&chars, j) { s += 15; }
        s -= (j - pos).min(10) as i32;
        prev = j;
        pos = j + 1;
    }
    Some(300 + s - len / 4)
}

/// `items` matching `query`, best first (ties by name), at most `limit`.
pub fn rank(query: &str, items: &[Match], limit: usize) -> Vec<Match> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(i32, &Match)> = items.iter().filter_map(|m| score(&q, &m.name).map(|s| (s, m))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, m)| m.clone()).collect()
}

pub fn request(id: u64, query: &str) -> Value {
    json!({"type": QUERY, "request_id": id, "query": query, "limit": LIMIT})
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub request_id: u64,
    pub query: String,
    pub hash: String,
    pub count: u64,
    pub partial: bool,
    pub matches: Vec<Match>,
}

/// Validate an `object_names` message (bounded: at most [`LIMIT`] matches, names up to 256 bytes).
pub fn parse(msg: &Value) -> Option<Response> {
    if msg["type"] != RESPONSE { return None; }
    let matches = msg["matches"].as_array()?.iter().take(LIMIT).filter_map(|m| {
        let name = m["name"].as_str().filter(|s| !s.is_empty() && s.len() <= MAX_NAME_BYTES)?;
        Some(Match { name: name.to_owned(), kind: m["type"].as_str().unwrap_or("").chars().take(32).collect(), mesh: m["mesh"].as_bool().unwrap_or(false) })
    }).collect();
    Some(Response {
        request_id: msg["request_id"].as_u64()?,
        query: msg["query"].as_str().unwrap_or("").to_owned(),
        hash: msg["hash"].as_str().unwrap_or("").chars().take(64).collect(),
        count: msg["count"].as_u64().unwrap_or(0),
        partial: msg["partial"].as_bool().unwrap_or(false),
        matches,
    })
}

/// One field's search state (UI thread). Pure: time is passed in, messages are returned/accepted.
#[derive(Default, Debug, Clone)]
pub struct Search {
    next_id: u64,
    pending: Option<(u64, String)>,
    pending_since: f64,
    typed_at: Option<f64>,
    query: String,
    hash: String,
    cache: HashMap<String, Vec<Match>>,
    pub results: Vec<Match>,
    pub partial: bool,
    pub count: Option<u64>,
}

impl Search {
    /// Focus (or the first frame of editing): ask at once for the current text.
    pub fn focus(&mut self, query: &str, now: f64) {
        self.query = query.to_owned();
        self.typed_at = Some(now - DEBOUNCE);
    }
    /// The text changed: re-rank what we have instantly, ask Blender after the debounce.
    pub fn edit(&mut self, query: &str, now: f64) {
        if query == self.query { return; }
        self.query = query.to_owned();
        self.typed_at = Some(now);
        self.results = rank(query, &self.results, LIMIT);
        if let Some(hit) = self.cache.get(query) { self.results = hit.clone(); self.partial = false; }
    }
    /// True while a keystroke waits out the debounce before it is sent.
    pub fn debouncing(&self) -> bool { self.typed_at.is_some() }
    /// Seconds until an outstanding query is abandoned, measured from `now`; None when nothing is pending.
    pub fn expires_in(&self, now: f64) -> Option<f64> {
        self.pending.as_ref().map(|_| (self.pending_since + TIMEOUT - now).max(0.))
    }
    /// Abandon an outstanding query that has waited past TIMEOUT. Returns true when it did.
    pub fn expire(&mut self, now: f64) -> bool {
        if self.pending.is_some() && now - self.pending_since >= TIMEOUT {
            self.pending = None;
            self.partial = false;
            return true;
        }
        false
    }
    /// "searching..." while a request for the current text is outstanding or debouncing.
    pub fn searching(&self) -> bool {
        self.typed_at.is_some() || self.pending.as_ref().is_some_and(|(_, q)| *q == self.query)
    }
    /// The request to send now, if the debounce elapsed and the answer is not cached.
    pub fn poll(&mut self, now: f64) -> Option<Value> {
        let typed = self.typed_at?;
        if now - typed < DEBOUNCE { return None; }
        self.typed_at = None;
        if let Some(hit) = self.cache.get(&self.query) {
            self.results = hit.clone();
            self.partial = false;
            // Still ask (cheap, bounded) so a changed scene refreshes the cache by hash.
        }
        self.next_id += 1;
        self.pending = Some((self.next_id, self.query.clone()));
        self.pending_since = now;
        Some(request(self.next_id, &self.query))
    }
    /// Apply a response; false when stale (an older request id) and discarded.
    pub fn accept(&mut self, response: Response) -> bool {
        let Some((id, query)) = &self.pending else { return false };
        if response.request_id != *id { return false; }
        let query = query.clone();
        if response.hash != self.hash { self.cache.clear(); self.hash = response.hash.clone(); }
        self.results = response.matches;
        self.partial = response.partial;
        self.count = Some(response.count);
        if !response.partial {
            self.pending = None;
            if self.cache.len() >= CACHE_ENTRIES { self.cache.clear(); }
            self.cache.insert(query, self.results.clone());
        }
        true
    }
}

/// Dropdown keyboard model: Up/Down move the highlight (wrapping), Enter picks it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key { Up, Down, Enter }
pub fn navigate(highlight: usize, len: usize, key: Key) -> (usize, Option<usize>) {
    if len == 0 { return (0, None); }
    let h = highlight.min(len - 1);
    match key {
        Key::Down => ((h + 1) % len, None),
        Key::Up => ((h + len - 1) % len, None),
        Key::Enter => (h, Some(h)),
    }
}

/// Link threads <-> UI: capable peers, queued queries, received responses.
#[derive(Default)]
pub struct Hub {
    pub capable_peers: usize,
    pub outgoing: Vec<Value>,
    pub incoming: Vec<Value>,
    /// Called when a reply arrives on a link thread, so the UI repaints to show it.
    pub wake: Option<Box<dyn Fn() + Send>>,
}
pub static HUB: Mutex<Hub> = Mutex::new(Hub { capable_peers: 0, outgoing: Vec::new(), incoming: Vec::new(), wake: None });

/// Install the repaint callback once (the UI thread calls this while drawing).
pub fn set_wake(f: impl Fn() + Send + 'static) {
    if let Ok(mut h) = HUB.lock() { if h.wake.is_none() { h.wake = Some(Box::new(f)); } }
}
/// Wake the UI for a new reply. Runs on the link thread; the callback only requests a repaint.
pub fn wake() {
    if let Ok(h) = HUB.lock() { if let Some(f) = &h.wake { f(); } }
}

pub fn available() -> bool { HUB.lock().map(|h| h.capable_peers > 0).unwrap_or(false) }
/// Queue a query for the capable link connection (dropped if none is connected).
pub fn send(query: Value) {
    if let Ok(mut h) = HUB.lock() {
        if h.capable_peers > 0 { h.outgoing.retain(|m| m["type"] != QUERY); h.outgoing.push(query); }
    }
}
pub fn take_incoming() -> Vec<Value> { HUB.lock().map(|mut h| std::mem::take(&mut h.incoming)).unwrap_or_default() }

#[cfg(test)]
mod tests {
    use super::*;
    fn m(name: &str) -> Match { Match { name: name.into(), kind: "MESH".into(), mesh: true } }

    #[test]
    fn scoring_prefers_exact_prefix_word_starts_and_short_names() {
        assert_eq!(score("cube", "Cube"), Some(1000));
        assert!(score("cu", "Cube").unwrap() > score("cu", "Cube.001").unwrap());
        assert!(score("cu", "Cube.001").unwrap() > score("cu", "BigCube").unwrap(), "prefix beats substring");
        assert!(score("cu", "BigCube").unwrap() > score("cu", "Biscuit").unwrap(), "word start beats mid-word");
        assert!(score("bc", "Big_Cube").unwrap() > score("bc", "abacus").unwrap(), "word-start subsequence");
        assert_eq!(score("xyz", "Cube"), None);
        assert_eq!(score("", "Cube"), Some(0));
        let ranked = rank("cu", &[m("Biscuit"), m("Cube.001"), m("Cube"), m("Camera"), m("BigCube")], 3);
        assert_eq!(ranked.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Cube", "Cube.001", "BigCube"]);
    }

    #[test]
    fn an_unanswered_query_expires_so_the_field_stops_waiting() {
        let mut s = Search::default();
        s.focus("cu", 0.0);
        assert!(s.poll(1.0).is_some());
        assert!(s.searching());
        assert!(s.expires_in(1.0).is_some_and(|left| (left - TIMEOUT).abs() < 1e-9));
        assert!(!s.expire(TIMEOUT - 0.5), "not yet");
        assert!(s.expire(1.0 + TIMEOUT));
        assert!(!s.searching() && s.expires_in(5.0).is_none());
    }

    #[test]
    fn framing_is_bounded_and_validated() {
        let q = request(7, "cu");
        assert_eq!((q["type"].as_str(), q["request_id"].as_u64(), q["limit"].as_u64()), (Some(QUERY), Some(7), Some(LIMIT as u64)));
        let many: Vec<Value> = (0..80).map(|i| json!({"name":format!("o{i}"),"type":"EMPTY","mesh":false})).collect();
        let mut msg = json!({"type":RESPONSE,"request_id":7,"query":"o","hash":"h","count":100000,"partial":true,"matches":many});
        let r = parse(&msg).unwrap();
        assert_eq!((r.matches.len(), r.partial, r.count), (LIMIT, true, 100000));
        msg["matches"] = json!([{"name":""},{"name":"x".repeat(300)},{"name":"Ok","type":"MESH","mesh":true}]);
        assert_eq!(parse(&msg).unwrap().matches, vec![m("Ok")]);
        assert!(parse(&json!({"type":"other"})).is_none());
        assert!(parse(&json!({"type":RESPONSE,"matches":[]})).is_none(), "request id required");
    }

    #[test]
    fn debounce_stale_discard_and_hash_cache() {
        let mut s = Search::default();
        s.focus("", 0.0);
        let first = s.poll(0.0).unwrap();
        assert_eq!(first["query"], "");
        s.edit("c", 0.01); s.edit("cu", 0.05);
        assert!(s.poll(0.10).is_none(), "debounced");
        assert!(s.searching());
        let second = s.poll(0.20).unwrap();
        assert_eq!((second["query"].as_str(), second["request_id"].as_u64()), (Some("cu"), Some(2)));
        let resp = |id: u64, q: &str, hash: &str, names: &[&str], partial: bool| Response { request_id: id, query: q.into(), hash: hash.into(), count: 3, partial, matches: names.iter().map(|n| m(n)).collect() };
        assert!(!s.accept(resp(1, "", "h1", &["Old"], false)), "stale id discarded");
        assert!(s.results.is_empty());
        assert!(s.accept(resp(2, "cu", "h1", &["Cube"], true)));
        assert!(s.partial && s.searching());
        assert!(s.accept(resp(2, "cu", "h1", &["Cube", "Cube.001"], false)));
        assert!(!s.searching());
        // Instant local re-rank while typing, then cache hit when returning to "cu".
        s.edit("cube.", 1.0);
        assert_eq!(s.results, vec![m("Cube.001")]);
        s.edit("cu", 1.05);
        assert_eq!(s.results.len(), 2);
        // A new scene hash clears the cache.
        let id = s.poll(2.0).unwrap()["request_id"].as_u64().unwrap();
        assert!(s.accept(resp(id, "cu", "h2", &["Cup"], false)));
        assert_eq!(s.cache.len(), 1);
    }

    #[test]
    fn dropdown_navigation_wraps_and_picks() {
        assert_eq!(navigate(0, 3, Key::Down), (1, None));
        assert_eq!(navigate(2, 3, Key::Down), (0, None));
        assert_eq!(navigate(0, 3, Key::Up), (2, None));
        assert_eq!(navigate(9, 3, Key::Enter), (2, Some(2)));
        assert_eq!(navigate(0, 0, Key::Enter), (0, None));
    }
}
