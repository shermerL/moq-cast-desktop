use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::Ipv4Addr,
    time::{Duration, Instant},
};

const SERVICE_TYPE: &str = "_moq._udp.local.";
const MAX_INSTANCE_LEN: usize = 64;
const MAX_DEVICES: usize = 128;
const MAX_DESCRIPTOR_ENDPOINTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PresenceState {
    Online,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct Presence {
    pub(crate) stable_id: String,
    pub(crate) display_name: String,
    pub(crate) state: PresenceState,
    pub(crate) watchable: bool,
}

impl fmt::Debug for Presence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Presence")
            .field("display_name", &self.display_name)
            .field("state", &self.state)
            .field("watchable", &self.watchable)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Lifecycle {
    Starting,
    Browsing,
    Degraded,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) revision: u64,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) devices: Vec<Presence>,
    watch_targets: BTreeMap<String, WatchTarget>,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("revision", &self.revision)
            .field("lifecycle", &self.lifecycle)
            .field("devices", &self.devices)
            .finish()
    }
}

impl Snapshot {
    pub(crate) fn starting() -> Self {
        Self {
            revision: 0,
            lifecycle: Lifecycle::Starting,
            devices: Vec::new(),
            watch_targets: BTreeMap::new(),
        }
    }

    pub(crate) fn watch_target(&self, stable_id: &str) -> Option<&WatchTarget> {
        self.watch_targets.get(stable_id)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct WatchTarget {
    pub(crate) port: u16,
    pub(crate) fingerprint: String,
    pub(crate) credential: String,
    pub(crate) addresses: Vec<Ipv4Addr>,
}

impl fmt::Debug for WatchTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WatchTarget")
            .field("endpoint_count", &self.addresses.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct ResolvedRecord {
    pub(crate) fullname: String,
    pub(crate) port: u16,
    pub(crate) fingerprint: Option<String>,
    pub(crate) credential: Option<String>,
    pub(crate) has_shared_secret: bool,
    pub(crate) addresses: Vec<Ipv4Addr>,
}

impl ResolvedRecord {
    #[cfg(test)]
    fn presence_only(fullname: &str) -> Self {
        Self {
            fullname: fullname.to_owned(),
            port: 0,
            fingerprint: None,
            credential: None,
            has_shared_secret: false,
            addresses: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistryChange {
    Added,
    Updated,
    Removed,
    Unchanged,
}

enum Entry {
    Found {
        expires_at: Instant,
    },
    Resolved {
        resolved: Box<ResolvedEntry>,
        expires_at: Instant,
    },
}

#[derive(Eq, PartialEq)]
struct ResolvedEntry {
    presence: Presence,
    metadata: ResolvedMetadata,
    watch_target: Option<WatchTarget>,
}

#[derive(Eq, PartialEq)]
struct ResolvedMetadata {
    instance: String,
    port: u16,
    fingerprint: Option<String>,
    credential: Option<String>,
    has_shared_secret: bool,
    addresses: Vec<Ipv4Addr>,
}

impl ResolvedMetadata {
    fn from_record(instance: String, record: ResolvedRecord) -> Self {
        Self {
            instance,
            port: record.port,
            fingerprint: record.fingerprint,
            credential: record.credential,
            has_shared_secret: record.has_shared_secret,
            addresses: record
                .addresses
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Registry {
    pub(crate) fn found(
        &mut self,
        fullname: &str,
        now: Instant,
        lease: Duration,
    ) -> RegistryChange {
        let Some(service) = validated_service(fullname) else {
            return RegistryChange::Unchanged;
        };
        let expires_at = now + lease;
        let has_capacity = self.entries.len() < MAX_DEVICES;
        match self.entries.get_mut(&service.key) {
            Some(Entry::Found {
                expires_at: current,
            })
            | Some(Entry::Resolved {
                expires_at: current,
                ..
            }) => {
                *current = expires_at;
                RegistryChange::Unchanged
            }
            None if !has_capacity => RegistryChange::Unchanged,
            None => {
                self.entries
                    .insert(service.key, Entry::Found { expires_at });
                RegistryChange::Unchanged
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn resolved(
        &mut self,
        fullname: &str,
        now: Instant,
        lease: Duration,
    ) -> RegistryChange {
        self.resolved_record(ResolvedRecord::presence_only(fullname), now, lease)
    }

    pub(crate) fn resolved_record(
        &mut self,
        record: ResolvedRecord,
        now: Instant,
        lease: Duration,
    ) -> RegistryChange {
        let Some(service) = validated_service(&record.fullname) else {
            return RegistryChange::Unchanged;
        };
        let metadata = ResolvedMetadata::from_record(service.instance, record);
        let watch_target = validated_watch_target(&metadata);
        let resolved = ResolvedEntry {
            presence: Presence {
                watchable: watch_target.is_some(),
                ..service.presence
            },
            metadata,
            watch_target,
        };
        let expires_at = now + lease;
        let has_capacity = self.entries.len() < MAX_DEVICES;

        match self.entries.get_mut(&service.key) {
            Some(Entry::Resolved {
                resolved: current,
                expires_at: current_expiry,
            }) if current.as_ref() == &resolved => {
                *current_expiry = expires_at;
                RegistryChange::Unchanged
            }
            Some(entry) => {
                let was_resolved = matches!(entry, Entry::Resolved { .. });
                *entry = Entry::Resolved {
                    resolved: Box::new(resolved),
                    expires_at,
                };
                if was_resolved {
                    RegistryChange::Updated
                } else {
                    RegistryChange::Added
                }
            }
            None if !has_capacity => RegistryChange::Unchanged,
            None => {
                self.entries.insert(
                    service.key,
                    Entry::Resolved {
                        resolved: Box::new(resolved),
                        expires_at,
                    },
                );
                RegistryChange::Added
            }
        }
    }

    pub(crate) fn removed(&mut self, fullname: &str) -> RegistryChange {
        let Some(service) = validated_service(fullname) else {
            return RegistryChange::Unchanged;
        };
        match self.entries.remove(&service.key) {
            Some(Entry::Resolved { .. }) => RegistryChange::Removed,
            Some(Entry::Found { .. }) | None => RegistryChange::Unchanged,
        }
    }

    pub(crate) fn expire(&mut self, now: Instant) -> RegistryChange {
        let before = self.online_count();
        self.entries.retain(|_, entry| match entry {
            Entry::Found { expires_at } | Entry::Resolved { expires_at, .. } => *expires_at > now,
        });
        if self.online_count() < before {
            RegistryChange::Removed
        } else {
            RegistryChange::Unchanged
        }
    }

    pub(crate) fn snapshot(&self, revision: u64, lifecycle: Lifecycle) -> Snapshot {
        let mut devices = Vec::new();
        let mut watch_targets = BTreeMap::new();
        for entry in self.entries.values() {
            let Entry::Resolved { resolved, .. } = entry else {
                continue;
            };
            devices.push(resolved.presence.clone());
            if let Some(target) = &resolved.watch_target {
                watch_targets.insert(resolved.presence.stable_id.clone(), target.clone());
            }
        }
        Snapshot {
            revision,
            lifecycle,
            devices,
            watch_targets,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    fn online_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry, Entry::Resolved { .. }))
            .count()
    }
}

fn canonical_fullname(fullname: &str) -> String {
    fullname.to_ascii_lowercase()
}

struct ValidatedService {
    key: String,
    instance: String,
    presence: Presence,
}

fn validated_service(fullname: &str) -> Option<ValidatedService> {
    let (instance, service_type) = fullname.split_once('.')?;
    if !service_type.eq_ignore_ascii_case(SERVICE_TYPE) {
        return None;
    }
    if instance.is_empty()
        || instance.len() > MAX_INSTANCE_LEN
        || !instance
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }

    let key = canonical_fullname(fullname);
    let stable_id = instance.to_ascii_lowercase();
    let suffix_start = stable_id.len().saturating_sub(6);
    let display_name = format!("MoQ device {}", &stable_id[suffix_start..]);
    Some(ValidatedService {
        key,
        instance: instance.to_owned(),
        presence: Presence {
            stable_id,
            display_name,
            state: PresenceState::Online,
            watchable: false,
        },
    })
}

fn validated_watch_target(metadata: &ResolvedMetadata) -> Option<WatchTarget> {
    if metadata.has_shared_secret
        || !is_hex(&metadata.instance, 16)
        || metadata
            .instance
            .bytes()
            .any(|byte| byte.is_ascii_uppercase())
        || metadata.port == 0
    {
        return None;
    }
    let fingerprint = metadata
        .fingerprint
        .as_ref()
        .filter(|value| is_hex(value, 64))?;
    let credential = metadata
        .credential
        .as_ref()
        .filter(|value| is_hex(value, 32))?;
    let addresses = metadata
        .addresses
        .iter()
        .copied()
        .filter(|address| {
            !address.is_loopback() && !address.is_unspecified() && !address.is_multicast()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_DESCRIPTOR_ENDPOINTS)
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return None;
    }
    Some(WatchTarget {
        port: metadata.port,
        fingerprint: fingerprint.clone(),
        credential: credential.clone(),
        addresses,
    })
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
fn sanitized_presence(fullname: &str) -> Option<Presence> {
    validated_service(fullname).map(|service| service.presence)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const LEASE: Duration = Duration::from_secs(30);
    const FULLNAME: &str = "peer_a._moq._udp.local.";

    fn fullname(index: usize) -> String {
        format!("peer_{index:03}._moq._udp.local.")
    }

    fn watch_record(
        fullname: &str,
        port: u16,
        fingerprint: Option<&str>,
        credential: Option<&str>,
        has_shared_secret: bool,
        addresses: Vec<Ipv4Addr>,
    ) -> ResolvedRecord {
        ResolvedRecord {
            fullname: fullname.to_owned(),
            port,
            fingerprint: fingerprint.map(str::to_owned),
            credential: credential.map(str::to_owned),
            has_shared_secret,
            addresses,
        }
    }

    fn valid_watch_record(fullname: &str, address: Ipv4Addr) -> ResolvedRecord {
        watch_record(
            fullname,
            4443,
            Some(&"a".repeat(64)),
            Some(&"b".repeat(32)),
            false,
            vec![address],
        )
    }

    #[test]
    fn found_is_pending_until_resolved() {
        let now = Instant::now();
        let mut registry = Registry::default();

        assert_eq!(
            registry.found(FULLNAME, now, LEASE),
            RegistryChange::Unchanged
        );
        assert!(registry.snapshot(1, Lifecycle::Browsing).devices.is_empty());
        assert_eq!(
            registry.resolved(FULLNAME, now, LEASE),
            RegistryChange::Added
        );
        assert_eq!(registry.snapshot(2, Lifecycle::Browsing).devices.len(), 1);
    }

    #[test]
    fn repeated_resolve_is_deduplicated() {
        let now = Instant::now();
        let mut registry = Registry::default();

        assert_eq!(
            registry.found(FULLNAME, now, LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(
            registry.found(FULLNAME, now + Duration::from_secs(1), LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(
            registry.resolved(FULLNAME, now, LEASE),
            RegistryChange::Added
        );
        assert_eq!(
            registry.resolved(FULLNAME, now + Duration::from_secs(1), LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(registry.snapshot(2, Lifecycle::Browsing).devices.len(), 1);
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn invalid_found_and_resolved_records_never_consume_capacity() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let overlong = format!("{}._moq._udp.local.", "a".repeat(MAX_INSTANCE_LEN + 1));

        for invalid in [
            "._moq._udp.local.",
            "bad name._moq._udp.local.",
            "peer._http._tcp.local.",
            overlong.as_str(),
        ] {
            assert_eq!(
                registry.found(invalid, now, LEASE),
                RegistryChange::Unchanged
            );
            assert_eq!(
                registry.resolved(invalid, now, LEASE),
                RegistryChange::Unchanged
            );
        }

        assert!(registry.entries.is_empty());
        assert!(registry.snapshot(1, Lifecycle::Browsing).devices.is_empty());

        assert_eq!(
            registry.resolved(FULLNAME, now, LEASE),
            RegistryChange::Added
        );
        assert_eq!(
            registry.resolved("bad name._moq._udp.local.", now, LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(registry.entries.len(), 1);
        assert!(registry.entries.contains_key(&canonical_fullname(FULLNAME)));
    }

    #[test]
    fn registry_keeps_existing_entries_when_capacity_is_reached() {
        let now = Instant::now();
        let mut registry = Registry::default();

        for index in 0..MAX_DEVICES {
            assert_eq!(
                registry.resolved(&fullname(index), now, LEASE),
                RegistryChange::Added
            );
        }
        let overflow = fullname(MAX_DEVICES);

        assert_eq!(
            registry.found(&overflow, now, LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(
            registry.resolved(&overflow, now, LEASE),
            RegistryChange::Unchanged
        );
        assert_eq!(registry.entries.len(), MAX_DEVICES);
        assert_eq!(
            registry.snapshot(1, Lifecycle::Browsing).devices.len(),
            MAX_DEVICES
        );
        assert!(
            !registry
                .entries
                .contains_key(&canonical_fullname(&overflow))
        );
        assert!(
            registry
                .entries
                .contains_key(&canonical_fullname(&fullname(0)))
        );
    }

    #[test]
    fn lost_entry_releases_capacity_for_a_new_device() {
        let now = Instant::now();
        let mut registry = Registry::default();
        for index in 0..MAX_DEVICES {
            registry.resolved(&fullname(index), now, LEASE);
        }
        let removed = fullname(0);
        let replacement = fullname(MAX_DEVICES);

        assert_eq!(registry.removed(&removed), RegistryChange::Removed);
        assert_eq!(
            registry.resolved(&replacement, now, LEASE),
            RegistryChange::Added
        );
        assert_eq!(registry.entries.len(), MAX_DEVICES);
        assert!(!registry.entries.contains_key(&canonical_fullname(&removed)));
        assert!(
            registry
                .entries
                .contains_key(&canonical_fullname(&replacement))
        );
    }

    #[test]
    fn dns_names_are_deduplicated_case_insensitively() {
        let now = Instant::now();
        let mut registry = Registry::default();

        registry.resolved("PEER_A._moq._udp.local.", now, LEASE);
        registry.resolved(FULLNAME, now, LEASE);

        assert_eq!(registry.snapshot(2, Lifecycle::Browsing).devices.len(), 1);
        assert_eq!(
            registry.removed("Peer_A._MOQ._UDP.LOCAL."),
            RegistryChange::Removed
        );
    }

    #[test]
    fn removed_and_expired_presence_leave_the_snapshot() {
        let now = Instant::now();
        let mut registry = Registry::default();

        registry.resolved(FULLNAME, now, LEASE);
        assert_eq!(registry.removed(FULLNAME), RegistryChange::Removed);
        assert!(registry.snapshot(1, Lifecycle::Browsing).devices.is_empty());

        registry.resolved(FULLNAME, now, LEASE);
        assert_eq!(registry.expire(now + LEASE), RegistryChange::Removed);
        assert!(registry.snapshot(2, Lifecycle::Browsing).devices.is_empty());
    }

    #[test]
    fn resolved_presence_is_bounded_and_redacted() {
        let presence = sanitized_presence(FULLNAME).expect("valid service name");
        let rendered = format!("{presence:?}");

        assert_eq!(presence.stable_id, "peer_a");
        assert_eq!(presence.display_name, "MoQ device peer_a");
        for sensitive in ["credential", "nonce", "proof", "fingerprint", "endpoint"] {
            assert!(!rendered.contains(sensitive));
        }
        assert!(sanitized_presence("bad name._moq._udp.local.").is_none());
        assert!(sanitized_presence("peer._http._tcp.local.").is_none());
    }

    #[test]
    fn lifecycle_is_independent_from_presence() {
        let now = Instant::now();
        let mut registry = Registry::default();
        registry.resolved(FULLNAME, now, LEASE);

        let degraded = registry.snapshot(4, Lifecycle::Degraded);
        assert_eq!(degraded.lifecycle, Lifecycle::Degraded);
        assert_eq!(degraded.devices[0].state, PresenceState::Online);
    }

    #[test]
    fn android_linux_and_windows_open_cluster_shapes_are_watchable() {
        let now = Instant::now();
        let mut registry = Registry::default();
        for (fullname, address) in [
            (
                "0123456789abcdef._moq._udp.local.",
                Ipv4Addr::new(192, 168, 1, 10),
            ),
            (
                "1111222233334444._moq._udp.local.",
                Ipv4Addr::new(10, 0, 0, 20),
            ),
            (
                "aaaabbbbccccdddd._moq._udp.local.",
                Ipv4Addr::new(172, 16, 0, 30),
            ),
        ] {
            assert_eq!(
                registry.resolved_record(valid_watch_record(fullname, address), now, LEASE),
                RegistryChange::Added
            );
        }

        let snapshot = registry.snapshot(1, Lifecycle::Browsing);
        assert_eq!(snapshot.devices.len(), 3);
        assert!(snapshot.devices.iter().all(|device| device.watchable));
        assert_eq!(
            snapshot.watch_target("0123456789abcdef").unwrap().port,
            4443
        );
        assert_eq!(
            snapshot.watch_target("1111222233334444").unwrap().port,
            4443
        );
        assert_eq!(
            snapshot.watch_target("aaaabbbbccccdddd").unwrap().port,
            4443
        );
    }

    #[test]
    fn shared_secret_and_invalid_descriptor_fields_are_not_watchable() {
        let now = Instant::now();
        let fingerprint = "a".repeat(64);
        let credential = "b".repeat(32);
        let address = Ipv4Addr::new(192, 168, 1, 20);
        let cases = [
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&credential),
                true,
                vec![address],
            ),
            watch_record(
                "0123456789abcde._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdeF._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdeg._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                0,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                None,
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&"a".repeat(63)),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&format!("{}g", "a".repeat(63))),
                Some(&credential),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&fingerprint),
                None,
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&"b".repeat(31)),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&format!("{}z", "b".repeat(31))),
                false,
                vec![address],
            ),
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![Ipv4Addr::LOCALHOST],
            ),
        ];

        for (index, record) in cases.into_iter().enumerate() {
            let mut registry = Registry::default();
            assert_eq!(
                registry.resolved_record(record, now, LEASE),
                RegistryChange::Added,
                "case {index}"
            );
            let snapshot = registry.snapshot(1, Lifecycle::Browsing);
            assert!(!snapshot.devices[0].watchable, "case {index}");
            assert!(
                snapshot
                    .watch_target(&snapshot.devices[0].stable_id)
                    .is_none(),
                "case {index}"
            );
        }
    }

    #[test]
    fn descriptor_addresses_are_filtered_sorted_deduplicated_and_capped() {
        let now = Instant::now();
        let mut registry = Registry::default();
        let mut addresses = vec![
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::new(224, 0, 0, 1),
        ];
        addresses.extend(
            (1..=10)
                .rev()
                .map(|octet| Ipv4Addr::new(192, 168, 1, octet)),
        );
        addresses.push(Ipv4Addr::new(192, 168, 1, 3));

        registry.resolved_record(
            watch_record(
                "0123456789abcdef._moq._udp.local.",
                4443,
                Some(&"a".repeat(64)),
                Some(&"b".repeat(32)),
                false,
                addresses,
            ),
            now,
            LEASE,
        );

        let snapshot = registry.snapshot(1, Lifecycle::Browsing);
        assert_eq!(
            snapshot.watch_target("0123456789abcdef").unwrap().addresses,
            (1..=8)
                .map(|octet| Ipv4Addr::new(192, 168, 1, octet))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn resolved_updates_and_lost_remove_private_watch_data() {
        let now = Instant::now();
        let fullname = "0123456789abcdef._moq._udp.local.";
        let mut registry = Registry::default();
        let first = watch_record(
            fullname,
            4443,
            Some(&"a".repeat(64)),
            Some(&"b".repeat(32)),
            false,
            vec![Ipv4Addr::new(192, 168, 1, 20)],
        );
        let updated = watch_record(
            fullname,
            5555,
            Some(&"c".repeat(64)),
            Some(&"d".repeat(32)),
            false,
            vec![Ipv4Addr::new(192, 168, 1, 21)],
        );

        assert_eq!(
            registry.resolved_record(first, now, LEASE),
            RegistryChange::Added
        );
        assert_eq!(
            registry.resolved_record(updated, now, LEASE),
            RegistryChange::Updated
        );
        let snapshot = registry.snapshot(1, Lifecycle::Browsing);
        let target = snapshot.watch_target("0123456789abcdef").unwrap();
        assert_eq!(target.port, 5555);
        assert_eq!(target.fingerprint, "c".repeat(64));
        assert_eq!(target.credential, "d".repeat(32));
        assert_eq!(registry.removed(fullname), RegistryChange::Removed);
        let lost = registry.snapshot(2, Lifecycle::Browsing);
        assert!(lost.devices.is_empty());
        assert!(lost.watch_target("0123456789abcdef").is_none());
    }

    #[test]
    fn nonwatchable_records_retain_private_resolved_metadata() {
        let now = Instant::now();
        let fullname = "0123456789abcdef._moq._udp.local.";
        let fingerprint = "a".repeat(64);
        let credential = "b".repeat(32);
        let address = Ipv4Addr::new(192, 168, 1, 20);
        let mut registry = Registry::default();
        registry.resolved_record(
            watch_record(
                fullname,
                4443,
                Some(&fingerprint),
                Some(&credential),
                true,
                vec![address],
            ),
            now,
            LEASE,
        );

        let Entry::Resolved { resolved, .. } = registry
            .entries
            .get(&canonical_fullname(fullname))
            .expect("resolved entry")
        else {
            panic!("expected resolved entry");
        };
        assert_eq!(resolved.metadata.instance, "0123456789abcdef");
        assert_eq!(resolved.metadata.port, 4443);
        assert_eq!(
            resolved.metadata.fingerprint.as_deref(),
            Some(fingerprint.as_str())
        );
        assert_eq!(
            resolved.metadata.credential.as_deref(),
            Some(credential.as_str())
        );
        assert_eq!(resolved.metadata.addresses, vec![address]);
        assert!(resolved.metadata.has_shared_secret);
        assert!(!resolved.presence.watchable);
        assert!(resolved.watch_target.is_none());
    }

    #[test]
    fn debug_output_redacts_private_watch_data() {
        let now = Instant::now();
        let fullname = "0123456789abcdef._moq._udp.local.";
        let fingerprint = "a".repeat(64);
        let credential = "b".repeat(32);
        let address = Ipv4Addr::new(192, 168, 99, 42);
        let mut registry = Registry::default();
        registry.resolved_record(
            watch_record(
                fullname,
                4443,
                Some(&fingerprint),
                Some(&credential),
                false,
                vec![address],
            ),
            now,
            LEASE,
        );

        let snapshot = registry.snapshot(1, Lifecycle::Browsing);
        let rendered = format!(
            "{snapshot:?} {:?} {:?}",
            snapshot.devices[0],
            snapshot.watch_target("0123456789abcdef").unwrap()
        );
        for secret in [
            fullname,
            fingerprint.as_str(),
            credential.as_str(),
            "192.168.99.42",
            "4443",
            "/.cluster/",
            "moqcast.screen/",
        ] {
            assert!(!rendered.contains(secret), "Debug leaked {secret}");
        }
    }
}
