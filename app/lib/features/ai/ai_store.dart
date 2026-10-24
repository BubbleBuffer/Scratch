// AI on this device: the vault's MCP endpoint and AI tasks.
//
// - Tasks are rows of the synced "AI tasks" collection (vault_mcp::tasks).
//   "Ask AI" (the `/` menu, the selection toolbar, the block menu and the
//   page menu) and tasks assigned to the agent start one, on the synced
//   default runner ([AiStore.runsOn]; phones: always the Hub):
//   - this computer: a row pinned to this device, which runs it with its
//     agent and the synced model and tool set of `this_device`;
//   - the Hub: a hub job about the page (hub_jobs_client.dart), run with
//     the hub runner's model and tool set.
//   Either way the answer replaces the task's placeholder in the page.
//   Status (queued, running, done, failed, cancelled) is in the row, so
//   every device sees it.
// - The in-app agent (BubblePaw) is the only thing that answers: it runs
//   the task in an agent session of its own and writes the answer itself.
//   Rows that name a runner removed on 2026-10-22 (`claude`, `codex`,
//   `deepseek`) are answered by the agent too. Claude Code, Codex and
//   other apps reach the vault from outside, through the MCP endpoint or
//   agent tokens; the app never starts them.
// - The MCP endpoint is served on the configured address when enabled in
//   settings.
//
// One store per open vault ([AiStore.of]); boot attaches the device's
// settings and starts it.

import '../../core/debounce.dart';

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../core/paths.dart';
import '../../core/vault.dart';
import '../../src/rust/api/ai.dart';
import '../../src/rust/api/agent.dart' show aiRunTaskWithAgent;
import '../../state/app_state.dart';
import '../../addons/agent/api.dart';
import 'ai_settings.dart';
import 'hub_jobs_client.dart';

export '../../src/rust/api/ai.dart' show AiTask;

/// Task statuses (the "Status" select of the AI tasks collection).
abstract final class AiStatus {
  static const queued = 'queued';
  static const running = 'running';
  static const done = 'done';
  static const failed = 'failed';
  static const cancelled = 'cancelled';
  static const interrupted = 'interrupted';

  /// Whether a run with [status] has not ended yet.
  static bool active(String status) => status == queued || status == running;
}

/// Starts and stops the agent's runs for add-ons (a task assigned to the
/// agent): [AiStore] on the synced default runner.
abstract interface class AgentRuns {
  /// Starts a run of [prompt] about [page]; its answer goes after block
  /// [after] or at the page's end. Completes with the run's id.
  Future<String> ask(String prompt, String page, {String? after});

  /// Stops run [id] if it has not ended.
  Future<void> cancel(String id);

  /// Run [id] as this device has it (null: not synced here yet).
  Future<AiTask?> run(String id);
}

class AiStore extends ChangeNotifier implements AgentRuns {
  AiStore._(this.vault, this.feed, this._settings) {
    _settings.addListener(notifyListeners);
    feed?.addListener(_onChanges);
    _agents = AgentStore.attach(vault, _settings)..addListener(_onAgentChanged);
  }

  final Vault vault;
  final ChangeFeed? feed;
  AiSettings _settings;
  AiSettings get settings => _settings;
  late final AgentStore _agents;

  static final _stores = Expando<AiStore>();

  /// The store of [app]'s vault (created with in-memory settings if boot
  /// has not attached any).
  static AiStore of(AppState app) => forVault(app.vault, app.feed);

  /// The settings of [vault]'s store, if it has one.
  static AiSettings? settingsOf(Vault vault) => _stores[vault]?._settings;

  static AiStore forVault(Vault vault, [ChangeFeed? feed]) =>
      _stores[vault] ??= AiStore._(vault, feed, AiSettings());

  /// Gives [vault]'s store the device's [settings].
  static AiStore attach(Vault vault, ChangeFeed? feed, AiSettings settings) {
    final s = forVault(vault, feed);
    if (!identical(s._settings, settings)) {
      s._settings.removeListener(s.notifyListeners);
      s._settings = settings..addListener(s.notifyListeners);
      AgentStore.existing(vault)?.useSettings(settings);
    }
    return s;
  }

  /// Agents run on desktops only.
  static bool get canRun =>
      !kIsWeb && (defaultTargetPlatform == TargetPlatform.windows || defaultTargetPlatform == TargetPlatform.linux);

  /// Makes the hub jobs client (tests use a fake hub).
  @visibleForTesting
  static HubJobs Function(HubLink hub, String deviceId)? hubJobsOverride;

  /// A sync round before a hub job, so the hub has the page as it is here
  /// (boot sets it).
  Future<void> Function()? syncBeforeHubJob;

  NotesMcp? _mcp;
  String? mcpUrl;
  String? mcpError;
  bool _started = false, _disposed = false, _pumping = false;
  Timer? _debounce;

  /// The newest tasks first (loaded by [refresh]).
  List<AiTask> tasks = const [];

  /// The task this device is running.
  String? runningId;
  String? _collection;

  /// Starts the endpoint (if enabled), fails tasks this device left
  /// running, loads the task list and runs queued tasks.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    if (canRun) {
      if (settings.mcpEnabled) await _startMcp();
      try {
        await vault.run((v) => aiRecoverTasks(vault: v), writes: true);
      } catch (_) {}
    }
    await refresh();
    unawaited(pump());
  }

  bool get mcpRunning => mcpUrl != null;

  /// Whether this device runs tasks: its agent is on and can run them.
  bool get canAnswer => canRun && settings.answersAskAi && _agents.canRunTasks;

  void _onAgentChanged() {
    _notify();
    if (_started && _agents.canRunTasks) unawaited(pump());
  }

  // ---------------------------------------------------------------- MCP

  Future<String?> _startMcp() async {
    final addr = settings.mcpEnabled ? settings.mcpAddress : '127.0.0.1:0';
    try {
      final m = _mcp ??= NotesMcp(vault: vault.raw);
      mcpUrl = await m.start(addr: addr);
      mcpError = null;
    } catch (e) {
      mcpUrl = null;
      mcpError = '$e';
    }
    _notify();
    return mcpUrl;
  }

  Future<void> _stopMcp() async {
    await _mcp?.stop();
    mcpUrl = null;
    _notify();
  }

  /// Turns the endpoint on [settings.mcpAddress] on or off.
  Future<void> setMcpEnabled(bool on) async {
    settings.update((s) => s.mcpEnabled = on);
    await _stopMcp();
    if (on) await _startMcp();
  }

  Future<void> setMcpAddress(String addr) async {
    settings.update((s) => s.mcpAddress = addr);
    if (settings.mcpEnabled) {
      await _stopMcp();
      await _startMcp();
    }
  }

  /// The endpoint's bearer token.
  Future<String> token() => (_mcp ??= NotesMcp(vault: vault.raw)).token();

  /// A new token; AI apps set up with the old one must be updated.
  Future<String> rotateToken() async {
    final t = await (_mcp ??= NotesMcp(vault: vault.raw)).rotateToken();
    _notify();
    return t;
  }

  // -------------------------------------------------------------- tasks

  /// Where Ask AI and tasks assigned to the agent run: the synced default
  /// runner (device override, vault, built-in, reachable fallback; phones:
  /// the Hub).
  AgentHost get runsOn => _agents.defaultHost;

  /// The hub, when this device can create hub jobs.
  HubLink? get hubLink => _agents.hubLink;

  /// Starts [prompt] about [page] on [runsOn]; its placeholder goes after
  /// block [after] (an empty paragraph there becomes it) or at the page's
  /// end. Completes with the run's id (the AI tasks row, or the hub job,
  /// which is a row too once it syncs here).
  @override
  Future<String> ask(String prompt, String page, {String? after}) async {
    // The synced defaults as they are now (another device may have changed
    // them).
    await _agents.refreshVaultDefaults();
    if (runsOn == AgentHost.hub) {
      final job = await _hubJob(prompt, page, after);
      unawaited(refresh());
      return job.id;
    }
    // Resolve once at creation: new desktop rows always name this device.
    // A device without a local runner must use its hub instead.
    if (!canRun) throw 'This device needs a hub to run AI tasks.';
    final t = await vault.run(
        (v) => aiCreateTaskHere(vault: v, prompt: prompt, page: page, after: after),
        writes: true);
    await refresh();
    unawaited(pump());
    return t.id;
  }

  HubJobs _jobs(HubLink hub) =>
      (hubJobsOverride ?? (h, d) => HubJobsClient(hubUrl: h.url, token: h.token, deviceId: d))(hub, vault.deviceId);

  Future<HubJob> _hubJob(String prompt, String page, String? after) async {
    final hub = hubLink;
    if (hub == null) throw 'Sign in to your hub in Settings > Sync first: the agent runs there.';
    await vault.flush();
    final sync = syncBeforeHubJob;
    if (sync != null) {
      try {
        await sync();
      } catch (e) {
        AppLog.info('sync before a hub job failed: $e');
      }
    }
    return _jobs(hub).create(prompt: prompt, page: page, after: after);
  }

  @override
  Future<AiTask?> run(String id) async {
    try {
      return await vault.run((v) => aiGetTask(vault: v, id: id));
    } catch (_) {
      return null;
    }
  }

  /// Stops run [id]: a hub job on the hub, a task on this device here
  /// (the runner stops its engine session; a late answer is dropped).
  @override
  Future<void> cancel(String id) async {
    final t = await run(id);
    if (t != null && !AiStatus.active(t.status)) return;
    if (t == null || t.node == 'hub') {
      final hub = hubLink;
      if (hub == null) throw 'Sign in to your hub in Settings > Sync to stop the agent there.';
      await _jobs(hub).cancel(id);
    } else {
      await vault.run((v) => aiCancelTask(vault: v, id: id), writes: true);
    }
    await refresh();
  }

  Future<void> retry(String id) async {
    await vault.run((v) => aiRetryTask(vault: v, id: id), writes: true);
    await refresh();
    unawaited(pump());
  }

  Future<void> refresh() async {
    try {
      tasks = await vault.run((v) => aiListTasks(vault: v, limit: 50));
      _collection = await vault.run((v) => aiTasksCollection(vault: v));
    } catch (_) {}
    _notify();
  }

  void _onChanges(ChangeBatch b) {
    final watched = {?_collection, for (final t in tasks) t.id};
    final relevant = b.reset ||
        b.changes.any((c) => c.kind == ChangeKind.pageCreated || (c.page != null && watched.contains(c.page)));
    if (!relevant) return;
    _debounce?.cancel();
    _debounce = Timer(Debounce.testDelay(const Duration(milliseconds: 150)), () {
      if (_disposed) return;
      refresh();
      pump();
    });
  }

  /// Runs queued tasks one at a time while this device can answer.
  Future<void> pump() async {
    if (_pumping || _disposed) return;
    if (canRun && settings.answersAskAi) {
      await _agents.ensureOpen();
      if (_pumping || _disposed) return;
    }
    if (!canAnswer) return;
    _pumping = true;
    try {
      while (!_disposed && canAnswer) {
        final t = await vault.run((v) => aiClaimNext(vault: v), writes: true);
        if (t == null) break;
        runningId = t.id;
        await refresh();
        // Whatever runner the row names, the agent answers it.
        await _runWithAgent(t);
        if (_disposed) break;
        runningId = null;
        await refresh();
      }
    } catch (e) {
      AppLog.info('ai tasks stopped: $e');
    } finally {
      _pumping = false;
      runningId = null;
      _notify();
    }
  }

  /// Runs the claimed task [t] with the in-app agent, which writes the
  /// answer (or the error) into the task itself.
  Future<void> _runWithAgent(AiTask t) async {
    final agents = AgentStore.attach(vault, settings);
    final agent = await agents.ensureOpen();
    if (agent == null) {
      final why = agents.openError ?? 'The agent is off on this device.';
      await vault.run((v) => aiFinishTask(vault: v, id: t.id, ok: false, text: why), writes: true);
      return;
    }
    try {
      await vault.run((v) => aiRunTaskWithAgent(agent: agent, vault: v, taskId: t.id), writes: true);
    } catch (e) {
      await vault.run((v) => aiFinishTask(vault: v, id: t.id, ok: false, text: '$e'), writes: true);
    }
    unawaited(agents.refreshSessions());
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  /// Stops the endpoint, then disposes the store.
  Future<void> close() async {
    final m = _mcp;
    _mcp = null;
    await m?.stop();
    m?.dispose();
    dispose();
  }

  @override
  void dispose() {
    _disposed = true;
    _debounce?.cancel();
    feed?.removeListener(_onChanges);
    _settings.removeListener(notifyListeners);
    _agents.removeListener(_onAgentChanged);
    final m = _mcp;
    _mcp = null;
    if (m != null) m.stop().whenComplete(m.dispose);
    _stores[vault] = null;
    super.dispose();
  }
}
