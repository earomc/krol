//! Operator, whitelist and ban lists (vanilla: `UserList` and its subclasses,
//! `ServerConfigurationManager.allowUserToConnect`).
//!
//! Files: `ops.json`, `whitelist.json`, `banned-players.json`, `banned-ips.json`, written
//! like vanilla with pretty-printed Gson in `HashMap` order (keyed by UUID string or IP).

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local, Utc};
use krol_java::JavaHashMap;
use krol_java::gson::JsonValue;
use tracing::warn;

use crate::status::java_uuid_string;

/// `yyyy-MM-dd HH:mm:ss Z`, the date format of ban entries.
const FILE_DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S %z";

/// Default ban source and reason.
pub const DEFAULT_SOURCE: &str = "(Unknown)";
/// Default ban reason.
pub const DEFAULT_REASON: &str = "Banned by an operator.";

/// One entry type of a user list file.
pub trait ListEntry: Sized {
    /// The map key (UUID string or IP).
    fn key(&self) -> String;
    /// Serializes the entry.
    fn to_json(&self) -> JsonValue;
    /// Deserializes an entry; `None` drops it, as vanilla drops entries without a value.
    fn from_json(json: &JsonValue) -> Option<Self>;
    /// Whether the entry has expired at `now` (bans only).
    fn expired(&self, _now: DateTime<Utc>) -> bool {
        false
    }
}

fn parse_uuid(s: &str) -> Option<u128> {
    // Java's UUID.fromString accepts 5 dash-separated hex groups.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 || parts.iter().any(|p| p.is_empty() || p.len() > 16) {
        return None;
    }
    let p = |i: usize| u64::from_str_radix(parts[i], 16).ok();
    let most = (p(0)? << 32) | ((p(1)? & 0xffff) << 16) | (p(2)? & 0xffff);
    let least = ((p(3)? & 0xffff) << 48) | (p(4)? & 0xffff_ffff_ffff);
    Some((u128::from(most) << 64) | u128::from(least))
}

fn profile_from_json(json: &JsonValue) -> Option<(u128, String)> {
    let uuid = parse_uuid(json.get("uuid")?.as_str()?)?;
    let name = json.get("name")?.as_str()?.to_owned();
    Some((uuid, name))
}

/// An operator (`ops.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpEntry {
    /// Player UUID.
    pub uuid: u128,
    /// Player name.
    pub name: String,
    /// Permission level (1–4).
    pub level: i32,
    /// Whether the operator may join a full server.
    pub bypasses_player_limit: bool,
}

impl ListEntry for OpEntry {
    fn key(&self) -> String {
        java_uuid_string(self.uuid)
    }

    fn to_json(&self) -> JsonValue {
        JsonValue::object()
            .with("uuid", java_uuid_string(self.uuid))
            .with("name", self.name.as_str())
            .with("level", self.level)
            .with("bypassesPlayerLimit", self.bypasses_player_limit)
    }

    fn from_json(json: &JsonValue) -> Option<Self> {
        let (uuid, name) = profile_from_json(json)?;
        Some(Self {
            uuid,
            name,
            level: json.get("level").and_then(JsonValue::as_i64).unwrap_or(0) as i32,
            bypasses_player_limit: json
                .get("bypassesPlayerLimit")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
        })
    }
}

/// A whitelisted player (`whitelist.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhitelistEntry {
    /// Player UUID.
    pub uuid: u128,
    /// Player name.
    pub name: String,
}

impl ListEntry for WhitelistEntry {
    fn key(&self) -> String {
        java_uuid_string(self.uuid)
    }

    fn to_json(&self) -> JsonValue {
        JsonValue::object()
            .with("uuid", java_uuid_string(self.uuid))
            .with("name", self.name.as_str())
    }

    fn from_json(json: &JsonValue) -> Option<Self> {
        let (uuid, name) = profile_from_json(json)?;
        Some(Self { uuid, name })
    }
}

/// Common ban fields (vanilla: `BanEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanInfo {
    /// When the ban was created.
    pub created: DateTime<FixedOffset>,
    /// Who banned (default `(Unknown)`).
    pub source: String,
    /// When the ban ends; `None` = forever.
    pub expires: Option<DateTime<FixedOffset>>,
    /// Ban reason (default `Banned by an operator.`).
    pub reason: String,
}

impl BanInfo {
    fn from_json(json: &JsonValue) -> Self {
        let date = |key: &str| {
            json.get(key)
                .and_then(JsonValue::as_str)
                .and_then(|s| DateTime::parse_from_str(s, FILE_DATE_FORMAT).ok())
        };
        Self {
            // Unparsable creation dates become "now", like vanilla.
            created: date("created").unwrap_or_else(|| Local::now().fixed_offset()),
            source: json
                .get("source")
                .and_then(JsonValue::as_str)
                .unwrap_or(DEFAULT_SOURCE)
                .to_owned(),
            // "forever" (or anything unparsable) means no expiry.
            expires: date("expires"),
            reason: json
                .get("reason")
                .and_then(JsonValue::as_str)
                .unwrap_or(DEFAULT_REASON)
                .to_owned(),
        }
    }

    fn append_json(&self, obj: JsonValue) -> JsonValue {
        let fmt = |d: &DateTime<FixedOffset>| {
            d.with_timezone(&Local).format(FILE_DATE_FORMAT).to_string()
        };
        obj.with("created", fmt(&self.created))
            .with("source", self.source.as_str())
            .with(
                "expires",
                self.expires
                    .as_ref()
                    .map_or_else(|| "forever".to_owned(), fmt),
            )
            .with("reason", self.reason.as_str())
    }

    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.expires.is_some_and(|e| e < now)
    }

    /// The ban message vanilla shows on login.
    fn message(&self, prefix: &str) -> String {
        let mut s = format!("{prefix}\nReason: {}", self.reason);
        if let Some(expires) = &self.expires {
            s.push_str(&format!(
                "\nYour ban will be removed on {}",
                java_short_date(expires)
            ));
        }
        s
    }
}

/// Formats like `SimpleDateFormat("yyyy-MM-dd 'at' HH:mm:ss z")` in the local zone.
///
/// Known deviation: Java prints the zone's short name (e.g. `CET`); krol prints `UTC`
/// for a zero offset and `GMT+hh:mm` otherwise (Java's form for unnamed zones).
fn java_short_date(date: &DateTime<FixedOffset>) -> String {
    let local = date.with_timezone(&Local);
    let offset = local.offset().local_minus_utc();
    let zone = if offset == 0 {
        "UTC".to_owned()
    } else {
        let sign = if offset < 0 { '-' } else { '+' };
        let abs = offset.abs();
        format!("GMT{sign}{:02}:{:02}", abs / 3600, (abs / 60) % 60)
    };
    format!("{} {zone}", local.format("%Y-%m-%d at %H:%M:%S"))
}

/// A banned player (`banned-players.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerBan {
    /// Player UUID.
    pub uuid: u128,
    /// Player name.
    pub name: String,
    /// Ban details.
    pub info: BanInfo,
}

impl ListEntry for PlayerBan {
    fn key(&self) -> String {
        java_uuid_string(self.uuid)
    }

    fn to_json(&self) -> JsonValue {
        self.info.append_json(
            JsonValue::object()
                .with("uuid", java_uuid_string(self.uuid))
                .with("name", self.name.as_str()),
        )
    }

    fn from_json(json: &JsonValue) -> Option<Self> {
        let (uuid, name) = profile_from_json(json)?;
        Some(Self {
            uuid,
            name,
            info: BanInfo::from_json(json),
        })
    }

    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.info.expired(now)
    }
}

/// A banned IP address (`banned-ips.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpBan {
    /// The address as vanilla keys it (see [`ip_key`]).
    pub ip: String,
    /// Ban details.
    pub info: BanInfo,
}

impl ListEntry for IpBan {
    fn key(&self) -> String {
        self.ip.clone()
    }

    fn to_json(&self) -> JsonValue {
        self.info
            .append_json(JsonValue::object().with("ip", self.ip.as_str()))
    }

    fn from_json(json: &JsonValue) -> Option<Self> {
        Some(Self {
            ip: json.get("ip")?.as_str()?.to_owned(),
            info: BanInfo::from_json(json),
        })
    }

    fn expired(&self, now: DateTime<Utc>) -> bool {
        self.info.expired(now)
    }
}

/// The key vanilla uses for an IP ban lookup (`UserListIPBans.addressToString`): the
/// socket address's `toString()` after `/` and before the first `:`.
///
/// Vanilla quirk: for IPv6 this keeps only the first group (e.g. `0` for `::1`).
pub fn ip_key(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6
            .segments()
            .iter()
            .map(|s| format!("{s:x}"))
            .collect::<Vec<_>>()
            .join(":"),
    };
    host.split(':').next().unwrap_or("").to_owned()
}

/// The OS message Java puts in a `FileNotFoundException` for a missing file.
fn file_not_found_text() -> &'static str {
    if cfg!(windows) {
        "The system cannot find the file specified"
    } else {
        "No such file or directory"
    }
}

/// A user list file.
#[derive(Debug)]
pub struct UserList<E> {
    path: PathBuf,
    entries: JavaHashMap<E>,
    /// What the list is called in vanilla's log messages ("user banlist", ...).
    label: &'static str,
}

impl<E: ListEntry> UserList<E> {
    /// Loads `path` if it exists; failures are logged and leave the list empty.
    ///
    /// Known deviation: vanilla aborts startup on syntactically invalid JSON; krol logs a
    /// warning and continues with an empty list.
    pub fn load(path: impl Into<PathBuf>, label: &'static str) -> Self {
        let mut list = Self {
            path: path.into(),
            entries: JavaHashMap::new(),
            label,
        };
        if list.path.exists() {
            match std::fs::read_to_string(&list.path)
                .map_err(|e| e.to_string())
                .and_then(|text| JsonValue::parse(&text))
            {
                Ok(JsonValue::Array(items)) => {
                    for item in &items {
                        if let Some(entry) = E::from_json(item) {
                            list.entries.insert(entry.key(), entry);
                        }
                    }
                }
                Ok(JsonValue::Null) => {}
                Ok(_) => warn!("Failed to load {}: not a JSON array", list.label),
                Err(e) => warn!("Failed to load {}: {e}", list.label),
            }
        } else {
            // Vanilla opens the file unconditionally and logs the exception (its stack
            // trace lines are dropped by the parity normalizer).
            let name = list
                .path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            warn!(
                "Failed to load {}: 
java.io.FileNotFoundException: {name} ({})",
                list.label,
                file_not_found_text()
            );
        }
        list
    }

    /// Writes the list (pretty Gson, no trailing newline).
    pub fn save(&self) {
        let json = JsonValue::Array(self.entries.iter().map(|(_, e)| e.to_json()).collect());
        if let Err(e) = std::fs::write(&self.path, json.to_gson_pretty_string()) {
            warn!("Failed to save {}: {e}", self.label);
        }
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn remove_expired(&mut self) {
        let now = Utc::now();
        self.entries.retain(|_, e| !e.expired(now));
    }

    /// Looks up an entry, first dropping expired ones (vanilla: `getEntry`).
    pub fn get(&mut self, key: &str) -> Option<&E> {
        self.remove_expired();
        self.entries.get(key)
    }

    /// Whether `key` has an entry (vanilla: `hasEntry`).
    pub fn contains(&mut self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Adds or replaces an entry.
    pub fn add(&mut self, entry: E) {
        self.entries.insert(entry.key(), entry);
    }

    /// Removes an entry.
    pub fn remove(&mut self, key: &str) -> Option<E> {
        self.entries.remove(key)
    }

    /// Entries in file order.
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.entries.iter().map(|(_, e)| e)
    }
}

/// All four lists, managed like vanilla's `DedicatedPlayerList`.
#[derive(Debug)]
pub struct UserLists {
    /// `ops.json`.
    pub ops: UserList<OpEntry>,
    /// `whitelist.json`.
    pub whitelist: UserList<WhitelistEntry>,
    /// `banned-players.json`.
    pub banned_players: UserList<PlayerBan>,
    /// `banned-ips.json`.
    pub banned_ips: UserList<IpBan>,
    /// `white-list` from `server.properties`.
    pub whitelist_enforced: bool,
}

impl UserLists {
    /// Loads (and rewrites, like vanilla) the lists in `dir`.
    pub fn load(dir: &Path, whitelist_enforced: bool) -> Self {
        let banned_players = UserList::load(dir.join("banned-players.json"), "user banlist");
        banned_players.save();
        let banned_ips = UserList::load(dir.join("banned-ips.json"), "ip banlist");
        banned_ips.save();
        let ops = UserList::load(dir.join("ops.json"), "operators list");
        let whitelist = UserList::load(dir.join("whitelist.json"), "white-list");
        ops.save();
        if !whitelist.path().exists() {
            whitelist.save();
        }
        Self {
            ops,
            whitelist,
            banned_players,
            banned_ips,
            whitelist_enforced,
        }
    }

    /// Re-reads `whitelist.json` (vanilla `/whitelist reload`).
    pub fn reload_whitelist(&mut self) {
        let path = self.whitelist.path().to_path_buf();
        self.whitelist = UserList::load(path, "white-list");
    }

    /// Vanilla `allowUserToConnect`: the disconnect reason, or `None` to admit.
    pub fn allow_user_to_connect(
        &mut self,
        uuid: u128,
        addr: SocketAddr,
        online_players: i32,
        max_players: i32,
    ) -> Option<String> {
        let key = java_uuid_string(uuid);
        if let Some(ban) = self.banned_players.get(&key) {
            return Some(ban.info.message("You are banned from this server!"));
        }
        let can_join =
            !self.whitelist_enforced || self.ops.contains(&key) || self.whitelist.contains(&key);
        if !can_join {
            return Some("You are not white-listed on this server!".into());
        }
        if let Some(ban) = self.banned_ips.get(&ip_key(addr)) {
            return Some(
                ban.info
                    .message("Your IP address is banned from this server!"),
            );
        }
        let bypass = self
            .ops
            .get(&key)
            .is_some_and(|op| op.bypasses_player_limit);
        (online_players >= max_players && !bypass).then(|| "The server is full!".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTCH: u128 = 0x069a79f4_44e9_4726_a5be_fca90e38aaf5;

    fn addr() -> SocketAddr {
        "127.0.0.1:5000".parse().unwrap()
    }

    #[test]
    fn startup_writes_files_like_vanilla() {
        let dir = tempfile::tempdir().unwrap();
        let _ = UserLists::load(dir.path(), false);
        for f in [
            "ops.json",
            "whitelist.json",
            "banned-players.json",
            "banned-ips.json",
        ] {
            assert_eq!(std::fs::read_to_string(dir.path().join(f)).unwrap(), "[]");
        }
    }

    #[test]
    fn parses_leniently_and_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ops.json"),
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch"},{"name":"no-uuid"},{"uuid":"bad","name":"x"}]"#,
        )
        .unwrap();
        let lists = UserLists::load(dir.path(), false);
        let ops: Vec<_> = lists.ops.iter().collect();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].level, 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ops.json")).unwrap(),
            "[\n  {\n    \"uuid\": \"069a79f4-44e9-4726-a5be-fca90e38aaf5\",\n    \"name\": \"Notch\",\n    \"level\": 0,\n    \"bypassesPlayerLimit\": false\n  }\n]"
        );
    }

    #[test]
    fn connect_checks_in_vanilla_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("banned-players.json"),
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","created":"2020-01-01 00:00:00 +0000","source":"Server","expires":"forever","reason":"griefing"}]"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("banned-ips.json"),
            r#"[{"ip":"127.0.0.1","created":"2020-01-01 00:00:00 +0000","expires":"2000-01-01 00:00:00 +0000"}]"#,
        )
        .unwrap();
        let mut lists = UserLists::load(dir.path(), true);
        assert_eq!(
            lists.allow_user_to_connect(NOTCH, addr(), 0, 20).unwrap(),
            "You are banned from this server!\nReason: griefing"
        );
        let other = 0x1234;
        assert_eq!(
            lists.allow_user_to_connect(other, addr(), 0, 20).unwrap(),
            "You are not white-listed on this server!"
        );
        lists.whitelist_enforced = false;
        // The IP ban expired in 2000, so it is dropped.
        assert_eq!(lists.allow_user_to_connect(other, addr(), 0, 20), None);
        assert_eq!(
            lists.allow_user_to_connect(other, addr(), 20, 20).unwrap(),
            "The server is full!"
        );
        lists.ops.add(OpEntry {
            uuid: other,
            name: "Op".into(),
            level: 4,
            bypasses_player_limit: true,
        });
        assert_eq!(lists.allow_user_to_connect(other, addr(), 20, 20), None);
    }

    #[test]
    fn ip_keys_match_vanilla_quirk() {
        assert_eq!(ip_key("10.0.0.7:25565".parse().unwrap()), "10.0.0.7");
        assert_eq!(ip_key("[::1]:25565".parse().unwrap()), "0");
        assert_eq!(ip_key("[2001:db8::1]:1".parse().unwrap()), "2001");
    }

    #[test]
    fn uuid_parsing_like_java() {
        assert_eq!(
            parse_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5"),
            Some(NOTCH)
        );
        assert_eq!(
            parse_uuid("1-2-3-4-5"),
            Some(0x0000_0001_0002_0003_0004_0000_0000_0005)
        );
        assert_eq!(parse_uuid("069a79f444e94726a5befca90e38aaf5"), None);
    }
}
