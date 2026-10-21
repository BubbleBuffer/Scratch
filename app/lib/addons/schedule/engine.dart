// The engine behind the schedule UI. The schedule's runner decides who answers:
// - this computer's own runner (the node record `localNodeId` names) is called
//   through the vault's local schedule calls, which work without a hub;
// - a hub node is called through the hub's agent connection, opened with the
//   sync settings' hub URL and token (the same settings the agent add-on uses);
// - another desktop is never called from here: the view shows its synced
//   status instead.
// Preview always goes through the vault, so it needs neither a hub nor a runner.

import '../../core/vault.dart';
import '../../services/addon_services.dart' show AppSettings;
import '../../src/rust/api/agent.dart';
import 'backend.dart';
import 'format.dart' show decodeObject;
import 'local_runner.dart';

class EngineScheduleBackend extends ScheduleBackend {
  EngineScheduleBackend._(this._vault, this._settings);

  static final Map<Vault, EngineScheduleBackend> _byVault = {};

  /// The backend for [vault], shared by every schedule view on it, so one hub
  /// connection serves them all. It closes its connection with the vault. The
  /// first backend for a vault also starts this device's runner with the
  /// stored setting (see [startLocalRunnerOnce]).
  static EngineScheduleBackend forVault(Vault vault, AppSettings settings) =>
      _byVault.putIfAbsent(vault, () {
        final backend = EngineScheduleBackend._(vault, settings);
        vault.addCloseListener(() {
          _byVault.remove(vault);
          backend._closeHub();
        });
        startLocalRunnerOnce(vault, settings.scheduleRunsHere);
        return backend;
      });

  final Vault _vault;
  final AppSettings _settings;

  NotesAgent? _hub;
  String _hubKey = '';

  /// The hub from the sync settings, when it can be used: an `https://` URL
  /// with a device token. Null otherwise.
  ({String url, String token})? get _hubLink {
    final url = _settings.clientUrl?.trim() ?? '';
    final token = _settings.clientToken?.trim() ?? '';
    if (!url.toLowerCase().startsWith('https://') || token.isEmpty) {
      return null;
    }
    return (url: url, token: token);
  }

  Future<NotesAgent> _hubAgent() async {
    final link = _hubLink;
    if (link == null) {
      throw const ScheduleUnavailable(
        'No hub is connected. Add one in Sync settings.',
      );
    }
    final key = '${link.url}\n${link.token}';
    if (_hub != null && _hubKey == key) return _hub!;
    await _closeHub();
    final NotesAgent agent;
    try {
      agent = await NotesAgent.connect(
        url: link.url,
        device: _vault.deviceId,
        token: link.token,
      );
    } catch (e) {
      throw ScheduleUnavailable('Hub not reachable: ${_detail(e)}');
    }
    if (_hub != null) {
      // Another call connected first; keep its connection.
      agent.dispose();
      return _hub!;
    }
    _hub = agent;
    _hubKey = key;
    return agent;
  }

  Future<void> _closeHub() async {
    final agent = _hub;
    _hub = null;
    _hubKey = '';
    if (agent == null) return;
    try {
      await agent.closeEvents();
    } catch (_) {
      // Closing a dead connection has nothing to report.
    }
    agent.dispose();
  }

  /// True when [runnerId] is this computer's own runner.
  Future<bool> _isLocal(String runnerId) async =>
      runnerId == await _vault.localNodeId();

  /// The hub connection for a hub node. Any other node is refused with the
  /// reason: this device's runner is handled locally, and another desktop is
  /// never called.
  Future<NotesAgent> _hubFor(String runnerId) async {
    ScheduleNode? node;
    for (final n in await listNodes()) {
      if (n.id == runnerId) node = n;
    }
    if (node == null) {
      throw const ScheduleUnavailable(
        'The runner is not in this vault\'s node list.',
      );
    }
    if (node.kind == 'hub') return _hubAgent();
    if (node.deviceId == _vault.deviceId) {
      throw const ScheduleUnavailable(
        'This computer\'s schedule runner is not running yet.',
      );
    }
    throw const ScheduleUnavailable(
      'Another desktop runs this schedule. Its status is shown from the vault.',
    );
  }

  /// Runs one local schedule call; its failure becomes the reason shown.
  Future<T> _local<T>(Future<T> Function() call) async {
    try {
      return await call();
    } catch (e) {
      throw ScheduleUnavailable(_detail(e));
    }
  }

  @override
  Future<SchedulePreview> preview(String scheduleJson) async {
    final p = await _vault.schedulePreview(scheduleJson);
    final runs = <DateTime>[];
    for (final o in p.occurrences.take(3)) {
      final at = _date(o.occurrence);
      if (at != null) runs.add(at);
    }
    return SchedulePreview(
      ok: p.valid,
      reason: p.reason,
      currentHash: p.currentHash,
      nextRuns: runs,
      renderedTitle: p.occurrences.isEmpty ? '' : p.occurrences.first.title,
    );
  }

  @override
  Future<ScheduleLiveState> state(String pageId, String runnerId) async {
    if (await _isLocal(runnerId)) {
      return _live(await _local(() => _vault.localScheduleState(pageId)));
    }
    final agent = await _hubFor(runnerId);
    final AgentScheduleState s;
    try {
      s = await agent.scheduleState(pageId: pageId);
    } catch (e) {
      // The state is the health check: a failure here means the runner
      // cannot be reached, so the next call connects again.
      await _closeHub();
      throw ScheduleUnavailable('Hub not reachable: ${_detail(e)}');
    }
    return _live(s);
  }

  @override
  Future<ScheduleLiveState> accept(
    String pageId,
    String runnerId,
    String hash,
  ) async {
    if (await _isLocal(runnerId)) {
      return _live(
        await _local(() => _vault.localScheduleAccept(pageId, hash)),
      );
    }
    final agent = await _hubFor(runnerId);
    return _live(await agent.scheduleAccept(pageId: pageId, hash: hash));
  }

  @override
  Future<ScheduleLiveState> pause(
    String pageId,
    String runnerId,
    bool paused,
  ) async {
    if (await _isLocal(runnerId)) {
      return _live(
        await _local(() => _vault.localSchedulePause(pageId, paused)),
      );
    }
    final agent = await _hubFor(runnerId);
    return _live(await agent.schedulePause(pageId: pageId, paused: paused));
  }

  @override
  Future<ScheduleRunRecord> runNow(String pageId, String runnerId) async {
    if (await _isLocal(runnerId)) {
      return _run(await _local(() => _vault.localScheduleRunNow(pageId)));
    }
    final agent = await _hubFor(runnerId);
    return _run(await agent.scheduleRunNow(pageId: pageId));
  }

  /// Every node record: pages of type `node` (the hidden Nodes collection
  /// is left out of search, so the type filter finds only records).
  @override
  Future<List<ScheduleNode>> listNodes() async {
    final hits = await _vault.search('type:node', limit: 200);
    final out = <ScheduleNode>[];
    for (final hit in hits) {
      final fields = decodeObject(await _vault.typeData(hit.id, 'node'));
      if (fields == null) continue;
      final name = fields['name'];
      out.add(
        ScheduleNode(
          id: hit.id,
          deviceId: fields['device_id'] is String
              ? fields['device_id'] as String
              : '',
          name: name is String && name.trim().isNotEmpty ? name : hit.title,
          kind: fields['kind'] == 'hub' ? 'hub' : 'desktop',
          available: fields['available'] == true,
          lastSeen: _date(fields['last_seen']),
        ),
      );
    }
    return out;
  }

  static ScheduleLiveState _live(AgentScheduleState s) => ScheduleLiveState(
    accepted: s.accepted,
    hashMatches: s.hashMatches,
    paused: s.paused,
    runnerState: s.runnerState,
    currentHash: s.currentHash,
    lastRuns: [for (final r in s.lastRuns) _run(r)],
    nextRun: _date(s.nextRun),
    reason: s.refusalReason,
  );

  static ScheduleRunRecord _run(AgentScheduleRun r) => ScheduleRunRecord(
    occurrence: _date(r.occurrence),
    at: _date(r.at),
    result: r.result,
    page: r.page,
    error: r.error,
  );

  /// An RFC 3339 instant as local time, or null.
  static DateTime? _date(Object? v) =>
      v is String ? DateTime.tryParse(v)?.toLocal() : null;

  static String _detail(Object e) => e.toString().trim();
}
