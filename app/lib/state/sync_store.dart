// Sync state: the client that syncs this vault with a hub (background loop
// and its status), on desktop the hub embedded in the app, the vault's
// unresolved conflicts, and attachment fetching. Boot and the settings
// screen (features/sync, features/settings) start and stop them. Changes
// that sync brings in reach the UI through the change feed like any other
// write.

import 'dart:async';
import 'dart:io' show File;

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/vault.dart';
import '../features/sync/cert_trust.dart';

/// What the top-bar indicator shows.
enum SyncPhase {
  /// Neither a client nor the hub is running.
  off,

  /// The client's last sync succeeded (or, hub only, the hub is serving).
  synced,

  /// A sync is in progress (or the first one has not finished).
  syncing,

  /// The hub could not be reached.
  offline,

  /// The hub answered with an error, or the hub could not start.
  error,
}

class SyncStore extends ChangeNotifier {
  SyncStore(this.vault, [this.feed]) {
    feed?.addListener(_onChanges);
    vault.addCloseListener(_onVaultClosed);
  }

  final Vault vault;
  final ChangeFeed? feed;

  NotesSync? _client;
  String? _token;
  SyncOptions? _options;
  NotesHub? _hub;
  Timer? _statusTimer;
  final _conflictTimer = Debounce();
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;

  /// The background loop's state (default: not running).
  SyncStatus status = const SyncStatus(running: false, syncing: false, syncs: 0);

  /// The hub URL this vault syncs with, while connected.
  String? hubUrl;

  /// The embedded hub's bound address, while serving.
  String? hubAddress;

  /// Why the embedded hub could not start (cleared when it starts).
  String? hubError;

  List<PairedDevice> devices = const [];

  /// Unresolved conflicts in the whole vault, oldest first (loaded by
  /// [refreshConflicts], and again whenever the vault changes).
  List<Conflict> conflicts = const [];

  /// Recent resolutions, including consumed applications on entries reopened
  /// by undo. Journal repairs appear only here, including after undo.
  List<Conflict> conflictLog = const [];

  /// Attachments being downloaded by [fetchAllAttachments]: (done, total),
  /// null when idle.
  (int, int)? attachmentProgress;

  /// What the client accepts for an unknown-issuer certificate (the pin
  /// for [hubUrl]'s origin: its leaf fingerprint and issuer), while
  /// connected.
  List<TrustedHubCertificate> trusted = const [];

  /// The chain (leaf first) the hub presented when the last sync failed
  /// with an untrusted certificate; null otherwise.
  List<CertificateInfo>? untrustedChain;

  /// The same chain as DER, leaf first (null with [untrustedChain]).
  List<Uint8List>? untrustedChainDer;

  /// The DER of the certificate that issued the untrusted leaf, when the
  /// hub sent it (kept with the pin when the user trusts it).
  Uint8List? get untrustedIssuerDer {
    final d = untrustedChainDer;
    return d != null && d.length > 1 ? d[1] : null;
  }

  /// Builds the client; tests replace it with a fake.
  @visibleForTesting
  static NotesSync Function(NotesVault vault, String url, String token, List<TrustedHubCertificate> trusted)
      clientFactory = _newClient;

  static NotesSync _newClient(NotesVault vault, String url, String token, List<TrustedHubCertificate> trusted) =>
      NotesSync.withTrustedCertificates(vault: vault, url: url, token: token, trusted: trusted);

  bool get connected => _client != null;
  bool get hosting => hubAddress != null;

  /// The embedded hub is available on desktop only (tests switch the
  /// platform with debugDefaultTargetPlatformOverride).
  static bool get canHost => switch (defaultTargetPlatform) {
        TargetPlatform.windows || TargetPlatform.linux || TargetPlatform.macOS => true,
        _ => false,
      };

  /// The indicator's phase and, for offline and error, the message.
  (SyncPhase, String?) get phase {
    if (_client != null) {
      final s = status;
      final err = s.lastError;
      if (s.syncing) return (SyncPhase.syncing, null);
      if (err != null) {
        // An untrusted certificate is not a reachability problem: the hub
        // answered, and only trusting its certificate fixes it.
        final offline = err.startsWith('hub connection') && !isUntrustedCertificateError(err);
        return (offline ? SyncPhase.offline : SyncPhase.error, err);
      }
      if (s.lastOkMs == null) return (SyncPhase.syncing, null);
      return (SyncPhase.synced, null);
    }
    if (hubError != null) return (SyncPhase.error, hubError);
    if (hubAddress != null) return (SyncPhase.synced, null);
    return (SyncPhase.off, null);
  }

  /// The current failure is a certificate with an unknown issuer that is
  /// not trusted (or no longer matches the trusted one).
  bool get untrustedCertificate {
    final (p, m) = phase;
    return p == SyncPhase.error && _client != null && isUntrustedCertificateError(m);
  }

  /// The fingerprint named by the untrusted-certificate error.
  String? get untrustedFingerprint => untrustedCertificate ? untrustedFingerprintOf(status.lastError) : null;

  // ---- client ----

  /// Starts syncing with the hub at [url] using a pairing [token]. The hub
  /// may present a certificate with an unknown issuer that [trusted]
  /// vouches for (its issuer, or its SHA-256 fingerprint).
  Future<void> connect(String url, String token,
      {SyncOptions? options, List<TrustedHubCertificate> trusted = const []}) async {
    await disconnect();
    final c = clientFactory(vault.raw, url, token, trusted);
    _client = c;
    _token = token;
    _options = options;
    this.trusted = List.unmodifiable(trusted);
    hubUrl = url;
    await c.start(options: options ?? defaultSyncOptions());
    _statusTimer = Timer.periodic(Debounce.instant ? const Duration(milliseconds: 1) : const Duration(seconds: 1), (_) => _refreshStatus());
    _refreshStatus();
    notifyListeners();
  }

  /// Reconnects the current client (same hub, token and options) accepting
  /// [trusted] instead, so trusting or revoking a certificate applies now.
  Future<void> reconnect({required List<TrustedHubCertificate> trusted}) async {
    final url = hubUrl, token = _token;
    if (_client == null || url == null || token == null) return;
    await connect(url, token, options: _options, trusted: trusted);
  }

  Future<void> disconnect() async {
    _statusTimer?.cancel();
    _statusTimer = null;
    final c = _client;
    _client = null;
    _token = null;
    _options = null;
    trusted = const [];
    untrustedChain = null;
    untrustedChainDer = null;
    hubUrl = null;
    if (c != null) {
      await c.stop();
      c.dispose();
    }
    status = const SyncStatus(running: false, syncing: false, syncs: 0);
    if (!_inactive) notifyListeners();
  }

  /// Asks the background loop to sync now (for example on resume).
  void syncNow() {
    _client?.syncNow();
    _refreshStatus();
  }

  /// One complete sync, outside the background loop.
  Future<SyncReport?> syncOnce() async => _client?.syncOnce();

  /// Completes true once a sync round succeeds: the first pull, since a round
  /// pulls before it pushes. Completes false if the vault closes first. A hub
  /// that never answers leaves it pending.
  Future<bool> firstSyncOk() {
    if (status.lastOkMs != null) return Future.value(true);
    final done = Completer<bool>();
    void check() {
      if (done.isCompleted) return;
      if (status.lastOkMs != null) {
        done.complete(true);
      } else if (_inactive) {
        done.complete(false);
      }
    }
    addListener(check);
    return done.future.whenComplete(() => removeListener(check));
  }

  void _refreshStatus() {
    final c = _client;
    if (c == null || _inactive) return;
    final s = c.status();
    if (s != status) {
      status = s;
      final untrusted = isUntrustedCertificateError(s.lastError);
      untrustedChain = untrusted ? c.untrustedCertificate() : null;
      untrustedChainDer = untrusted ? c.untrustedCertificateDer() : null;
      notifyListeners();
    }
  }

  // ---- embedded hub (desktop) ----

  /// Serves this vault on [addr] (for example `0.0.0.0:47100`); returns the
  /// bound address. On failure [hubError] says why and this rethrows.
  Future<String> startHub(String addr) async {
    try {
      final h = _hub ??= await NotesHub.newInstance(vault: vault.raw);
      hubAddress = await h.start(addr: addr);
      hubError = null;
    } catch (e) {
      hubAddress = null;
      hubError = 'Could not start the hub on $addr: $e';
      notifyListeners();
      rethrow;
    }
    await refreshDevices();
    return hubAddress!;
  }

  Future<void> stopHub() async {
    await _hub?.stop();
    hubAddress = null;
    hubError = null;
    if (!_inactive) notifyListeners();
  }

  /// A new pairing token for a device named [label] (shown once).
  Future<String> pair(String label) async {
    final h = _hub ??= await NotesHub.newInstance(vault: vault.raw);
    final token = await h.pair(label: label);
    await refreshDevices();
    return token;
  }

  Future<int> unpair(String label) async {
    final h = _hub ??= await NotesHub.newInstance(vault: vault.raw);
    final n = await h.unpair(label: label);
    await refreshDevices();
    return n;
  }

  /// Reloads the paired devices (desktop; the token file is read even while
  /// the hub is stopped).
  Future<void> refreshDevices() async {
    if (!canHost) return;
    final h = _hub ??= await NotesHub.newInstance(vault: vault.raw);
    devices = await h.devices();
    if (!_inactive) notifyListeners();
  }

  /// A readable name for a device id: "This device", a paired device's
  /// label, or "Another device".
  String deviceName(String device) {
    if (device == vault.deviceId) return 'This device';
    for (final d in devices) {
      if (d.device == device) return d.label;
    }
    return 'Another device';
  }

  // ---- conflicts ----

  void _onChanges(ChangeBatch b) {
    if (_inactive) return;
    _conflictTimer.run(const Duration(milliseconds: 150), refreshConflicts);
  }

  Future<void> refreshConflicts() async {
    _conflictTimer.cancel();
    if (_inactive) return;
    final list = await vault.run((v) => v.conflicts(page: null));
    if (_inactive) return;
    final log = await vault.run((v) => v.conflictLog(page: null));
    if (_inactive) return;
    list.sort((a, b) => a.timeMs.compareTo(b.timeMs));
    log.sort((a, b) => b.timeMs.compareTo(a.timeMs));
    if (!listEquals(list.map(_key).toList(), conflicts.map(_key).toList()) ||
        !listEquals(log.map(_key).toList(), conflictLog.map(_key).toList())) {
      conflicts = list;
      conflictLog = log;
      // A paired device's id is known only after its first sync: reload the
      // devices when a side names one we don't know, so the label shows it.
      if (canHost && list.any((c) => c.sides.any((s) => !_knownDevice(s.device)))) {
        try {
          await refreshDevices();
        } catch (_) {} // the label falls back to "Another device"
        if (_inactive) return;
      }
      notifyListeners();
    }
  }

  bool _knownDevice(String device) =>
      device.isEmpty || device == vault.deviceId || devices.any((d) => d.device == device);

  static String _key(Conflict c) => '${c.page}/${c.id}/${c.timeMs}/${c.status}/${c.undoable}/'
      '${c.applied.map((a) => '${a.timeMs}:${a.undoneAt}').join(',')}';

  List<Conflict> conflictsOf(String page) => [for (final c in conflicts) if (c.page == page) c];

  List<Conflict> conflictLogOf(String page) => [for (final c in conflictLog) if (c.page == page) c];

  /// The pages with conflicts, in order of their first conflict.
  List<String> get conflictPages {
    final out = <String>[];
    for (final c in conflicts) {
      if (!out.contains(c.page)) out.add(c.page);
    }
    return out;
  }

  Future<void> resolve(Conflict c, Resolution r) async {
    await vault.run((v) => v.resolveConflict(page: c.page, id: c.id, resolution: r), writes: true);
    await refreshConflicts();
  }

  Future<void> undoConflict(Conflict c) async {
    await vault.run((v) => v.undoConflict(page: c.page, id: c.id), writes: true);
    await refreshConflicts();
  }

  // ---- attachments ----

  /// The local path of attachment [id]; downloads it from the hub first if
  /// this device does not have it yet. Null if it is missing and cannot be
  /// fetched (not connected, or the hub lacks it). The editor calls this
  /// for images and files.
  Future<String?> ensureAttachment(String id) async {
    try {
      final p = await vault.run((v) => v.attachmentPath(id: id));
      if (File(p).existsSync()) return p;
    } catch (_) {}
    final c = _client;
    if (c == null) return null;
    try {
      await c.fetchAttachment(id: id);
      return await vault.run((v) => v.attachmentPath(id: id));
    } catch (_) {
      return null;
    }
  }

  /// Attachments the vault's pages reference but this device lacks.
  Future<List<String>> missingAttachments() => vault.run((v) => v.missingAttachments());

  /// Downloads every missing attachment, one by one (progress in
  /// [attachmentProgress]). Returns how many were fetched.
  Future<int> fetchAllAttachments() async {
    final c = _client;
    if (c == null || attachmentProgress != null) return 0;
    final ids = await missingAttachments();
    var done = 0, ok = 0;
    attachmentProgress = (0, ids.length);
    notifyListeners();
    try {
      for (final id in ids) {
        if (_inactive || _client != c) break;
        try {
          await c.fetchAttachment(id: id);
          ok++;
        } catch (_) {}
        done++;
        attachmentProgress = (done, ids.length);
        if (!_inactive) notifyListeners();
      }
    } finally {
      attachmentProgress = null;
      if (!_inactive) notifyListeners();
    }
    return ok;
  }

  void _onVaultClosed() {
    _conflictTimer.cancel();
    _statusTimer?.cancel();
  }

  @override
  void dispose() {
    vault.removeCloseListener(_onVaultClosed);
    _disposed = true;
    feed?.removeListener(_onChanges);
    _statusTimer?.cancel();
    _conflictTimer.cancel();
    final c = _client;
    _client = null;
    if (c != null) c.stop().whenComplete(c.dispose);
    final h = _hub;
    _hub = null;
    if (h != null) h.stop().whenComplete(h.dispose);
    super.dispose();
  }
}
