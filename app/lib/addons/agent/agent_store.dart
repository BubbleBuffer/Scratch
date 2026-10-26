import '../../services/addon_services.dart';
// The in-app agent (BubblePaw over the open vault) for one vault: its chats,
// the open chat's state and the live event stream.
//
// - Local on Windows/Linux while "Use the agent" is on; always Hub mode on
//   Android, with browser sign-in in Settings > AI. `NotesAgent` opens on use or at
//   boot, and reopened when connection/tool settings change. The shipped
//   app always uses Bypass and preserves dormant permission preferences.
// - "Agent runs on" picks the host: this computer (`NotesAgent.open`) or the
//   hub this vault syncs with (`NotesAgent.connect`, with the sync
//   settings' hub URL and device token and this vault's device id). The hub
//   counts only when it is an `https://` hub with a token; otherwise the
//   desktop agent runs here. Android stays signed out until a Hub is available.
// - One subscription to the agent's events. For the open chat, durable
//   events newer than the snapshot reload its state; live paragraphs are
//   the reply being written. A snapshot's `live_seen` paragraphs at its
//   `seq` are already in its `streaming_text` and are skipped. A `gap`
//   catches up with `events_after`.
// - "Agent runs on" is the default host for new chats. The AI tab can
//   switch the whole tab (chat list, models, search) to the other host while
//   the open chat has no messages; "New chat" goes back to the default.
//   Each chat's host and picked model are saved per device
//   (`agent_chats.json`, [AgentChatPrefs]); opening a chat of the other host
//   switches the tab to it.
// - Closing ends the bridge's event stream first (`closeEvents`), so
//   cancelling the subscription returns at once.
//
// One store per open vault ([AgentStore.of]); boot attaches the device's
// AI settings. AppState.dispose closes it.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show PlatformInt64Util;

import '../../core/debounce.dart';
import '../../core/paths.dart';
import '../../core/tool_sets.dart';
import '../../core/vault.dart';
import '../../ui/app_name.dart';
import '../../src/rust/api/agent.dart' hide AgentPermissionMode;
import '../../src/rust/api/agent.dart' as bridge show AgentPermissionMode;
import '../../src/rust/api/vault.dart' as bridge
    show VaultAgentDefaults, VaultAgentModel;
import '../../state/app_state.dart';
import 'dev_replay.dart';
import '../../src/rust/api/types.dart' show TrustedHubCertificate;
import 'host_relay_link.dart';
import 'agent_chat_prefs.dart';
import 'agent_search_source.dart';
import 'agent_search_vault.dart';
import 'outline_names.dart';

export '../../src/rust/api/agent.dart'
    show
        AgentApproval,
        AgentContextChat,
        AgentContextPage,
        AgentContextSelection,
        AgentDiffHunk,
        AgentDiffLine,
        AgentEdit,
        AgentProject,
        AgentHostInfo,
        AgentHostRelayStatus,
        AgentMcpServer,
        AgentMessage,
        AgentMessageContext,
        AgentModel,
        AgentRunRecord,
        AgentSession,
        AgentSessionState,
        AgentToolCall,
        AgentToolActivity,
        AgentUiAction;

/// The agent's run statuses that mean a run is still going.
bool runActive(AgentRunRecord r) =>
    r.status == 'running' || r.status == 'pending';

const _paragraphGap = '\n\n';

/// Where a chat is shown: the dock beside the editor, or the full Agent screen.
/// The transcript of each presentation is mounted on its own, so a span can
/// name the one that should show it.
enum AgentPresentation { dock, screen }

/// A span of a chat to show: from its first to its last message id. A span
/// with a [presentation] is taken only by that presentation; a span without
/// one is taken by the first chat that shows it. The target lives here, in
/// memory, and not in a cached reference.
class AgentAnchor {
  const AgentAnchor({
    required this.sessionId,
    required this.firstMessageId,
    required this.lastMessageId,
    this.presentation,
  });

  final String sessionId;
  final String firstMessageId;
  final String lastMessageId;
  final AgentPresentation? presentation;
}

/// The hub this vault syncs with, as the agent reaches it.
@immutable
class HubLink {
  const HubLink(this.url, this.token);
  final String url, token;

  /// The sync settings' hub, when the agent can use it: an `https://` hub
  /// with a device token.
  static HubLink? of(AppSettings? s, {bool allowLoopback = false}) {
    final url = s?.clientUrl?.trim() ?? '',
        token = s?.clientToken?.trim() ?? '';
    final uri = Uri.tryParse(url);
    // Auth also accepts loopback HTTP for local, synthetic integration tests.
    final loopback =
        allowLoopback &&
        uri?.scheme == 'http' &&
        (uri?.host == '127.0.0.1' || uri?.host == 'localhost') &&
        uri?.userInfo.isEmpty == true &&
        uri?.hasQuery == false &&
        uri?.hasFragment == false;
    if ((!url.toLowerCase().startsWith('https://') && !loopback) ||
        token.isEmpty) {
      return null;
    }
    return HubLink(url, token);
  }
}

class AgentStore extends ChangeNotifier {
  AgentStore._(this.vault, this._settings, this._dataDir) {
    _settings.addListener(_onSettings);
    _key = _configKey();
    _hubKey = _hubConnectionKey();
    unawaited(_loadVaultDefaults());
  }

  final Vault vault;
  AiSettings _settings;
  AiSettings get settings => _settings;
  String? _dataDir;

  static final _stores = Expando<AgentStore>();

  /// The store of [app]'s vault (with in-memory settings if boot has not
  /// attached any).
  static AgentStore of(AppState app) => forVault(app.vault)
    .._appState = app
    ..useAppSettings(app.settings);

  static AgentStore forVault(Vault vault) => _stores[vault] ??= AgentStore._(
    vault,
    const AddonServices().aiSettingsOf(vault) ?? AiSettings(),
    null,
  );

  /// The open store of [vault], if any (without creating one).
  static AgentStore? existing(Vault vault) => _stores[vault];

  /// Gives [vault]'s store the device's [settings] and data folder (the
  /// agent keeps `agent/` in it; default: next to the settings file).
  static AgentStore attach(
    Vault vault,
    AiSettings settings, {
    String? dataDir,
  }) {
    final s = forVault(vault);
    if (dataDir != null && dataDir != s._dataDir) {
      s._dataDir = dataDir;
      s._chatPrefs = null;
      s._settingsChanged(deferNotify: true);
    }
    s.useSettings(settings);
    return s;
  }

  /// Follows [settings] from now on (the AI store's, so both agree).
  void useSettings(AiSettings settings) {
    if (identical(_settings, settings) || _disposed) return;
    _settings.removeListener(_onSettings);
    _settings = settings..addListener(_onSettings);
    // May run while widgets build: notify after the frame's work.
    _settingsChanged(deferNotify: true);
    if (settings.hasLegacyAgentDefaults) unawaited(_loadVaultDefaults());
  }

  // ------------------------------- synced default runner and model

  /// The vault's agent defaults as last read (synced settings page).
  VaultAgentDefaultsView _vaultDefaults = VaultAgentDefaultsView.none;
  VaultAgentDefaultsView get vaultDefaults => _vaultDefaults;
  Future<void>? _vaultDefaultsRead;

  /// Reads the vault's agent defaults again (after a sync, or when Settings
  /// or the tab opens). Copies this device's pre-sync values in once.
  Future<void> refreshVaultDefaults() async {
    final pending = _vaultDefaultsRead;
    if (pending != null) await pending;
    await _loadVaultDefaults();
  }

  Future<void> _loadVaultDefaults() => _vaultDefaultsRead ??= () async {
    try {
      await _migrateLegacyDefaults();
      final (d, sets) = await vault.run(
        (v) async => (
          await v.agentDefaults(),
          {
            for (final h in AgentHost.values)
              h: await v.agentToolSet(runner: h.runner),
          },
        ),
      );
      final view = VaultAgentDefaultsView(
        host: AgentHost.ofRunner(d.runner),
        models: {
          for (final m in d.models)
            ?AgentHost.ofRunner(m.runner): m.model,
        },
        toolSets: {
          for (final e in sets.entries)
            if (e.value != defaultAgentToolSet) e.key: e.value,
        },
      );
      if (_disposed || view == _vaultDefaults) return;
      _vaultDefaults = view;
      _settingsChanged(deferNotify: true);
    } catch (e) {
      debugPrint('agent defaults: $e');
    } finally {
      _vaultDefaultsRead = null;
    }
  }();

  /// Old builds kept "Agent runs on" and the default model on each device.
  /// They go into the vault once, only when it has none, and are then
  /// dropped here (not kept as overrides). Android's old host is ignored:
  /// it always ran on the Hub.
  Future<void> _migrateLegacyDefaults() async {
    final s = settings;
    if (!s.hasLegacyAgentDefaults) return;
    final legacyHost = hubAgentPlatform ? null : s.legacyAgentHost;
    final modelHost = legacyHost ?? builtInAgentHost;
    final model = s.legacyAgentModel;
    await vault.run(
      (v) => v.seedAgentDefaults(
        seed: bridge.VaultAgentDefaults(
          runner: legacyHost?.runner,
          models: [
            if (model != null)
              bridge.VaultAgentModel(runner: modelHost.runner, model: model),
          ],
        ),
      ),
      writes: true,
    );
    if (identical(s, settings)) {
      s.update((s) {
        s.legacyAgentHost = null;
        s.legacyAgentModel = null;
      });
    }
  }

  /// Sets the vault's default runner (synced to every device).
  Future<void> setVaultDefaultHost(AgentHost? h) async {
    await vault.run(
      (v) => v.setAgentDefaultRunner(runner: h?.runner),
      writes: true,
    );
    await refreshVaultDefaults();
  }

  /// Sets the vault's default model for [runner] (null: clear it).
  Future<void> setVaultDefaultModel(AgentHost runner, String? model) async {
    await vault.run(
      (v) => v.setAgentDefaultModel(runner: runner.runner, model: model),
      writes: true,
    );
    await refreshVaultDefaults();
  }

  /// Sets [runner]'s tool set in the vault (synced). Each runner reads it
  /// when its agent starts; this computer's agent restarts on the change.
  Future<void> setVaultToolSet(AgentHost runner, String set) async {
    await vault.run(
      (v) => v.setAgentToolSet(
        runner: runner.runner,
        set_: set == defaultAgentToolSet ? null : set,
      ),
      writes: true,
    );
    await refreshVaultDefaults();
  }

  /// The default runner here: the vault's "Run AI on", else the built-in
  /// default, then a reachable fallback ([ResolvedAgentHost.fellBack]).
  /// Settings no longer offer a per-device choice (2026-10-22), so a device
  /// override saved by older builds is not applied.
  ResolvedAgentHost get resolvedHost =>
      resolveAgentHost(vault: _vaultDefaults.host, usable: usableHosts);

  /// [runner]'s default model: the vault's value (null: the host's own).
  ({String? model, AgentDefaultSource? source}) resolvedModel(
    AgentHost runner,
  ) => resolveAgentModel(runner, vault: _vaultDefaults.models);

  /// Follows the app settings (the hub connection) from now on. Until
  /// then they are read from `settings.json` in the data folder.
  void useAppSettings(AppSettings app) {
    if (identical(_app, app) || _disposed) return;
    _app?.removeListener(_onSettings);
    _app = app..addListener(_onSettings);
    _settingsChanged(deferNotify: true);
  }

  AppSettings? _app;
  AppState? _appState;
  AppSettings? _appFromFile;

  AppSettings _appSettings() =>
      _app ??
      (_appFromFile ??= AppSettings.load(
        File('${_folder()}${Platform.pathSeparator}settings.json'),
      ));

  /// What this device trusts for the hub at [url] beyond the public roots
  /// (Settings > Sync > Trusted certificates), as sync does.
  List<TrustedHubCertificate> _hubTrust(String url) =>
      _appSettings().trustedCertificates.trustFor(url);

  /// The vault's hub when the agent can run there (null: it cannot).
  HubLink? get hubLink =>
      HubLink.of(_appSettings(), allowLoopback: hubAgentPlatform && kDebugMode);

  /// Where new chats run: [resolvedHost] (Android: always the Hub).
  AgentHost get defaultHost =>
      hubAgentPlatform ? AgentHost.hub : resolvedHost.host;

  /// The host the tab switched to away from [defaultHost] (null: none).
  AgentHost? _hostOverride;
  AgentHost? _seenDefault;

  /// Where the agent runs now: the tab's host.
  AgentHost get host {
    if (hubAgentPlatform) return AgentHost.hub;
    final o = _hostOverride;
    if (o == AgentHost.hub && hubLink != null) return AgentHost.hub;
    if (o == AgentHost.local) return AgentHost.local;
    return defaultHost;
  }

  /// The hosts chats can run on here: this computer, and the hub when the
  /// vault syncs with a usable one. Android: the Hub only.
  List<AgentHost> get usableHosts => hubAgentPlatform
      ? const [AgentHost.hub]
      : hubLink == null
      ? const [AgentHost.local]
      : AgentHost.values;

  /// Whether the AI tab shows the host picker.
  bool get canPickHost => usableHosts.length > 1;

  /// Whether the open chat keeps its host: it has messages (or is loading).
  /// A draft (no chat yet) or an empty chat can still switch.
  bool get hostLocked {
    if (openId == null) return false;
    final s = state;
    return s == null || s.messages.isNotEmpty;
  }

  AgentChatPrefs? _chatPrefs;
  AgentChatPrefs get _prefs => _chatPrefs ??= AgentChatPrefs.load(
    _dataDir == null && settings.file == null
        ? null
        : File('${_folder()}${Platform.pathSeparator}agent_chats.json'),
  );

  /// The host chat [id] was created on, if this device recorded it.
  AgentHost? hostOf(String id) => _prefs[id]?.host;

  /// Chats this store created that may still be empty.
  final Set<String> _created = {};

  final List<Future<bool> Function(AgentHost from)> _switchConfirms = [];

  /// Registers [ask], which asks the user before opening a chat on the
  /// other host stops a reply running on this computer. The latest one
  /// mounted asks; with none, the tab keeps its host.
  void addHostSwitchConfirm(Future<bool> Function(AgentHost from) ask) =>
      _switchConfirms.add(ask);

  void removeHostSwitchConfirm(Future<bool> Function(AgentHost from) ask) =>
      _switchConfirms.remove(ask);

  /// Closes [vault]'s store if it has one.
  static Future<void> closeFor(Vault vault) async => _stores[vault]?.close();

  /// The local agent runs on Windows and Linux; Android uses the Hub.
  static bool get canRun => agentPlatform;

  /// App tests: register the core's Confirm tool `test_confirm`. The shipped
  /// bridge uses Bypass, so it runs without approval.
  @visibleForTesting
  static bool testTools = false;

  /// Android always offers Hub chat, including before the first sign-in.
  bool get enabled =>
      const AddonServices().addonEnabled('agent') &&
      canRun &&
      (hubAgentPlatform || settings.agentEnabled);

  bool get requiresHubSignIn => hubAgentPlatform && hubLink == null;

  NotesAgent? _agent;
  Future<NotesAgent?>? _opening;
  StreamSubscription<AgentEvent>? _sub;
  StreamSubscription<AgentEvent>? _uiSub;
  NotesAgent? _uiAgent;
  final _uiActions = StreamController<AgentUiAction>.broadcast(sync: true);
  final _viewScope = Debounce();
  Future<void> _scopeWork = Future.value();
  int _visibleChats = 0;
  String? _subscribedSession;
  String? _liveActivitySummary;
  bool _disposed = false;
  String _key = '';
  String _hubKey = '';
  bool _resetHubState = false;
  final _reopen = Debounce();
  final _reload = Debounce();
  final _reloadList = Debounce();

  /// Why the agent could not open (shown in the chat).
  String? openError;

  /// Whether the agent is open.
  bool get isOpen => _agent != null && !_resetHubState;

  /// Every chat, newest first.
  List<AgentSession> sessions = const [];

  /// The title filter of the chat list.
  String filter = '';

  /// The chats matching [filter].
  List<AgentSession> get visibleSessions {
    final q = filter.trim().toLowerCase();
    if (q.isEmpty) return sessions;
    return [
      for (final s in sessions)
        if (s.title.toLowerCase().contains(q)) s,
    ];
  }

  /// The available models, the default first.
  List<AgentModel> models = const [];

  /// The models the open host reports. A chat keeps its current model
  /// available even when the host no longer lists it.
  List<String> shownModels({String? current}) => [
    ...allModelIds,
    if (current != null && !allModelIds.contains(current)) current,
  ];

  List<String> get allModelIds => {for (final m in models) m.id}.toList();

  String modelLabel(String id) {
    if (id.startsWith('claude-code/')) {
      return switch (id.substring('claude-code/'.length)) {
        'opus' => 'Opus (latest)',
        'sonnet' => 'Sonnet (latest)',
        'haiku' => 'Haiku (latest)',
        'claude-fable-5-1' => 'Fable 5.1',
        'claude-opus-5-5' => 'Opus 5.5',
        'claude-sonnet-5-5' => 'Sonnet 5.5',
        'claude-haiku-5-5' => 'Haiku 5.5',
        _ => _nameOf(id),
      };
    }
    return _nameOf(id);
  }

  String _nameOf(String id) {
    for (final list in [models, for (final c in _hostModels.values) c.models]) {
      final name = list.where((m) => m.id == id).firstOrNull?.name;
      if (name != null) return name;
    }
    return id;
  }

  // ------------------------------------------- Settings > Default model

  /// The model lists last read from each host, keyed by what they depend
  /// on ([_modelsKey]), so Settings can list [defaultHost]'s models while
  /// the tab is on the other host.
  final Map<AgentHost, _HostModels> _hostModels = {};
  final Set<AgentHost> _hostModelsLoading = {};
  Future<void>? _localModelsRead;
  bool _modelsFailed = false;

  /// Tests: reads a host's models instead of opening an agent there.
  @visibleForTesting
  static Future<List<AgentModel>> Function(AgentHost host)? debugHostModels;

  /// Tests: the host default reported with [debugHostModels]' lists.
  @visibleForTesting
  static String? debugHostDefaultModel;

  String _modelsKey(AgentHost h) => h == AgentHost.hub
      ? _hubConnectionKey()
      : '${settings.agentBunPath.trim()}\u0000'
            '${settings.agentClaudeCodeDir.trim()}';

  /// What Settings > Default model lists: [defaultHost]'s models, read from
  /// that host even when the tab is on the other one. `ids` is null while
  /// they load, or when the host can't list them (`unavailable`): the
  /// setting is then shown as stored, never rewritten.
  ({List<String>? ids, bool unavailable}) defaultHostModels() {
    final r = runnerModels(defaultHost);
    if (r.live) {
      return (ids: {for (final m in r.models) m.id}.toList(), unavailable: false);
    }
    return (ids: null, unavailable: r.unreachable);
  }

  /// Settings > Run AI on: [runner]'s models and its own default model.
  /// `live` when read from that runner now. Otherwise, for the hub, the last
  /// list it reported on this device ([AiSettings.hubModels]), with
  /// `unreachable` once it can't be reached; neither while a read runs.
  RunnerModels runnerModels(AgentHost runner) {
    RunnerModels known({required bool unreachable}) {
      final cache = runner == AgentHost.hub ? settings.hubModels : null;
      return RunnerModels(
        models: [
          for (final m in cache?.models ?? const <({String id, String name})>[])
            AgentModel(id: m.id, name: m.name),
        ],
        hostDefault: cache?.defaultModel,
        unreachable: unreachable,
      );
    }

    if (runner == AgentHost.hub && hubLink == null) {
      return known(unreachable: true);
    }
    if (runner == host) {
      if (isOpen && models.isNotEmpty) {
        return RunnerModels(
          models: models,
          hostDefault: hostInfo?.defaultModel,
          live: true,
        );
      }
      // An open tab that couldn't list them. A failed open says nothing
      // about the list: read it on its own connection below.
      if (isOpen && _modelsFailed) return known(unreachable: true);
    }
    final cached = _hostModels[runner];
    if (cached != null && cached.key == _modelsKey(runner)) {
      if (cached.failed) return known(unreachable: true);
      return RunnerModels(
        models: cached.models,
        hostDefault: cached.hostDefault,
        live: true,
      );
    }
    _readHostModels(runner);
    return known(unreachable: false);
  }

  /// Keeps the hub's list for when it can't be reached.
  void _rememberHubModels(List<AgentModel> list, String? hostDefault) {
    if (list.isEmpty) return;
    final next = HubModelList(
      models: [for (final m in list) (id: m.id, name: m.name)],
      defaultModel: hostDefault,
    );
    if (next == settings.hubModels) return;
    settings.update((s) => s.hubModels = next);
  }

  /// Lets Settings try a host that couldn't list its models again.
  void retryDefaultHostModels() {
    _hostModels.removeWhere((_, c) => c.failed);
  }

  void _readHostModels(AgentHost h) {
    if (_disposed || !_hostModelsLoading.add(h)) return;
    final key = _modelsKey(h);
    final read = () async {
      // Called from build: start the bridge calls after it.
      await Future<void>.value();
      var list = const <AgentModel>[];
      String? hostDefault;
      var failed = false;
      try {
        final debug = debugHostModels;
        if (debug != null) {
          list = await debug(h);
          hostDefault = debugHostDefaultModel;
        } else {
          (list, hostDefault) = await _listModelsOn(h);
        }
      } catch (e) {
        failed = true;
        AppLog.info('agent models on ${h.name}: $e');
      }
      _hostModelsLoading.remove(h);
      if (_disposed) return;
      if (_modelsKey(h) == key) {
        _hostModels[h] = _HostModels(
          key,
          list,
          failed: failed,
          hostDefault: hostDefault,
        );
        if (h == AgentHost.hub && !failed) {
          _rememberHubModels(list, hostDefault);
        }
      }
      _notify();
    }();
    if (h == AgentHost.local) {
      _localModelsRead = read.whenComplete(() => _localModelsRead = null);
    }
  }

  /// Opens a short-lived agent on [h] for its model list and default.
  Future<(List<AgentModel>, String?)> _listModelsOn(AgentHost h) async {
    final hub = h == AgentHost.hub ? hubLink : null;
    if (h == AgentHost.hub && hub == null) throw StateError('no hub');
    final agent = await _count(() => _connect(hub));
    try {
      final info = await _count(agent.hostInfo);
      return (
        _withExtras(await _count(agent.models), info),
        info.defaultModel,
      );
    } finally {
      await agent.closeEvents();
      agent.dispose();
    }
  }

  /// Adds the Claude Code models a host with that provider always offers.
  static List<AgentModel> _withExtras(
    List<AgentModel> models,
    AgentHostInfo? info,
  ) {
    if (!(info?.providers.contains('claude-code') ?? false)) return models;
    const extras = {
      'claude-code/claude-fable-5-1': 'Fable 5.1',
      'claude-code/claude-opus-5-5': 'Opus 5.5',
      'claude-code/claude-sonnet-5-5': 'Sonnet 5.5',
      'claude-code/claude-haiku-5-5': 'Haiku 5.5',
    };
    return [
      ...models,
      for (final entry in extras.entries)
        if (!models.any((m) => m.id == entry.key))
          AgentModel(id: entry.key, name: entry.value),
    ];
  }

  /// Capabilities reported by the connected host; unknown until it opens.
  AgentHostInfo? hostInfo;

  /// Detected Claude overrides, shown only for the local host.
  AgentClaudeDetection? claudeDetection;

  List<AgentProject> projects = const [];

  /// Effective policy of the shipped app; the saved preference is dormant.
  AgentPermissionMode get permissionMode => AgentPermissionMode.bypass;

  bool get canRunTasks => hostInfo?.canRunTasks ?? false;

  /// The host and tools shown in settings, using the host's capabilities.
  String? get hostDescription {
    final info = hostInfo;
    if (info == null) return null;
    final where = info.kind == 'desktop'
        ? 'This computer'
        : Uri.tryParse(hubLink?.url ?? '')?.host ?? '';
    final tools = info.packs.contains('desktop')
        ? 'vault, files & shell'
        : 'vault';
    return 'Runs on: $where · tools: $tools';
  }

  @visibleForTesting
  void debugSetHostInfo(AgentHostInfo info) {
    hostInfo = info;
    _notify();
  }

  /// The open chat and its state (null until loaded).
  String? openId;
  AgentSessionState? state;

  /// The reply being written in the open chat (live paragraphs).
  String streaming = '';

  /// The last command's error in the open chat (shown inline).
  String? error;

  /// Pending approvals added by widget tests as fake state.
  final List<AgentApproval> _testApprovals = [];

  /// Live paragraph events received for the open chat since its snapshot.
  final List<AgentEvent> _live = [];

  /// The open chat's snapshot position, and the last durable sequence seen.
  int _snapSeq = 0, _seenSeq = 0;
  bool _loading = false, _reloadAgain = false;

  /// The open chat's chapters and sections. Loaded after each snapshot;
  /// [_outlineLoads] numbers the loads so a late one cannot overwrite a newer.
  ChatOutline outline = ChatOutline(chapters: const []);

  /// The chat whose outline [outline] holds, once that load has finished.
  String? outlineFor;

  /// The message list the outline was loaded for. The outline only changes
  /// with messages, so a snapshot that adds none does not load it again.
  String? _outlineKey;
  int _outlineLoads = 0;

  /// The open chat's outline revision as of its last outline load. A changed
  /// revision reloads the outline even when no message changed.
  BigInt? _outlineRevision;
  Timer? _outlinePoll;
  bool _outlinePolling = false;

  /// How often a visible chat asks for its outline revision.
  static const outlinePollInterval = Duration(milliseconds: 500);

  /// Where search rows go, per vault. Production writes them to the vault's
  /// index; tests replace it and restore it afterwards. Null indexes nothing.
  static AgentSearchSink Function(Vault vault)? searchSinkFor =
      VaultAgentSearchSink.new;

  AgentSearchIndex? _search;
  Timer? _searchPoll;
  bool _agentOnHub = false;

  /// How often the chat list and every chat's outline revision are swept into
  /// the search index while the agent is open.
  static const searchSweepInterval = Duration(seconds: 5);

  /// Reads the outline revision of a chat. Tests replace it to fake changes.
  @visibleForTesting
  static Future<BigInt> Function(NotesAgent agent, String sessionId)?
  debugRevisionLoader;

  static Future<BigInt> _defaultRevision(NotesAgent a, String id) =>
      a.sessionOutlineRevision(sessionId: id);
  AgentAnchor? _pendingAnchor;

  /// A span to show once its chat has loaded, taken by the open chat view.
  AgentAnchor? get pendingAnchor => _pendingAnchor;

  /// The pending span, when [by] presentation is the one that shows it.
  AgentAnchor? pendingFor(AgentPresentation by) {
    final a = _pendingAnchor;
    return a != null && (a.presentation == null || a.presentation == by)
        ? a
        : null;
  }

  /// Returns the span [pendingFor] gives [by] and clears it, so it is shown
  /// once. Another presentation's span stays pending.
  AgentAnchor? takePendingAnchor(AgentPresentation by) {
    final a = pendingFor(by);
    if (a != null) _pendingAnchor = null;
    return a;
  }

  final Map<String, String> _modelBySession = {};

  ReplayPlayback? replay;
  Set<String> _userNamed = {};
  String displaySessionTitle(String id, String raw) =>
      _shownTitle(onHub: host == AgentHost.hub, id: id, raw: raw);

  /// The chat title the search index writes. It follows the backend the open
  /// agent actually runs on ([_agentOnHub]), not the platform, so switching
  /// platform while the agent stays open cannot relabel its search rows.
  String _searchTitle(AgentSession s) =>
      _shownTitle(onHub: _agentOnHub, id: s.id, raw: s.title);

  String _shownTitle({
    required bool onHub,
    required String id,
    required String raw,
  }) => onHub || _userNamed.contains(id) ? raw : outlineNameGist(raw, '').name;

  /// Loads the dev provider, then drives recorded users through ordinary send.
  Future<ReplayPlayback> startReplay(String path, {int delayMs = 300}) async {
    if (!devReplayEnabled || host == AgentHost.hub) {
      throw StateError(
        'Live replay needs NOTES_DEV_REPLAY=1 and a local agent.',
      );
    }
    final a = await ensureOpen();
    if (a == null) throw StateError(openError ?? 'The agent is off.');
    await replay?.stop();
    replay?.removeListener(_notify);
    replay?.dispose();
    replay = null;
    final session = await createSession(title: 'Live replay');
    if (session == null) throw StateError('Could not create the replay chat.');
    final loaded = await _count(
      () => a.replayLoad(
        sessionId: session.id,
        path: path,
        delayMs: BigInt.from(delayMs),
      ),
    );
    pickModel(session.id, 'replay/replay');
    final playback = ReplayPlayback(
      this,
      a,
      session.id,
      loaded.userTexts,
      Duration(milliseconds: delayMs),
    );
    replay = playback;
    playback.addListener(_notify);
    _notify();
    playback.start();
    return playback;
  }

  /// Paragraph events received (tests check that a reply streamed).
  @visibleForTesting
  int paragraphsSeen = 0;

  /// The model for chat [id] (null: the next new chat): the one picked
  /// for it, else [defaultModel].
  String? modelOf(String? id) {
    final picked =
        _modelBySession[id ?? ''] ?? (id == null ? null : _prefs[id]?.model);
    if (picked != null) return picked;
    return defaultModel;
  }

  /// The default model on the connected host: the "Default model" setting
  /// when this host offers it, else the host's first model.
  String? get defaultModel {
    final def = resolvedModel(host).model ?? '';
    final ids = allModelIds;
    if (ids.isEmpty) return def.isEmpty ? null : def;
    return ids.contains(def) ? def : ids.first;
  }

  void pickModel(String? id, String model) {
    _modelBySession[id ?? ''] = model;
    if (id != null) _prefs.pickModel(id, host, model);
    _notify();
  }

  /// Switches the tab to [h] for the open chat while it has no messages
  /// and no reply running. An empty chat this store created moves: it is
  /// removed here and a new one is started on [h].
  Future<void> pickHost(AgentHost h) async {
    if (hostLocked || running || h == host || !usableHosts.contains(h)) {
      return;
    }
    final draft = openId;
    if (draft != null &&
        _created.contains(draft) &&
        (state?.messages.isEmpty ?? false)) {
      await deleteSession(draft);
    }
    await _switchHost(h);
    if (draft != null) await createSession();
  }

  /// Starts a chat on the default host and opens it ("New chat").
  Future<AgentSession?> newChat() async {
    // Another device may have changed the synced default since.
    await refreshVaultDefaults();
    await _switchHost(defaultHost);
    return createSession();
  }

  /// Closes the agent and opens it on [h]: chat list, models and search
  /// follow. A hub reply keeps running on the hub.
  Future<void> _switchHost(AgentHost h) async {
    if (_disposed || h == host || !usableHosts.contains(h)) return;
    _seenDefault = defaultHost;
    _hostOverride = h == defaultHost ? null : h;
    _modelBySession.remove('');
    _key = _configKey();
    _reopen.cancel();
    hostInfo = null;
    await _close(cancelOpen: !_agentOnHub);
    _notify();
    if (enabled) await ensureOpen();
    _notify();
  }

  /// Pending approvals of the open chat.
  List<AgentApproval> get pendingApprovals => [
    ...?state?.pendingApprovals,
    ..._testApprovals,
  ];

  /// Whether the open chat has a run going.
  bool get running => state?.runs.any(runActive) ?? false;

  /// Page actions are delivered only while a chat is on screen.
  Stream<AgentUiAction> get uiActions => _uiActions.stream;
  String? get visibleSessionId => _visibleChats > 0 ? openId : null;
  String? get latestActivitySummary {
    final current = state?.runs.where(runActive).lastOrNull;
    return _liveActivitySummary ??
        state?.activity
            .where((a) => current == null || a.runId == current.id)
            .lastOrNull
            ?.summary;
  }

  /// A shared chat view registers its lifetime, independently of the host.
  void showSession(bool visible) {
    _visibleChats = (_visibleChats + (visible ? 1 : -1)).clamp(0, 100);
    _scheduleViewScope();
    _syncOutlinePoll();
  }

  void _scheduleViewScope() {
    if (_disposed) return;
    _viewScope.run(const Duration(milliseconds: 1), () {
      _scopeWork = _scopeWork.then((_) => _count(_syncViewScope)).catchError((
        Object e,
      ) {
        _subscribedSession = null;
        AppLog.info('agent viewing subscription: $e');
      });
    });
  }

  Future<void> _syncViewScope() async {
    final agent = _agent, id = visibleSessionId;
    if (_disposed || agent == null || _subscribedSession == id) return;
    _subscribedSession = id;
    final previous = _uiSub;
    _uiSub = null;
    // Cancel immediately; closing the Rust stream below makes cancellation
    // finish even when there are no more events.
    final cancelled = previous?.cancel();
    final viewer = _uiAgent;
    _uiAgent = null;
    if (viewer != null) {
      await viewer.closeEvents();
      viewer.dispose();
    }
    if (host == AgentHost.local) {
      final global = _sub;
      final globalCancelled = global?.cancel();
      await agent.closeEvents();
      if (globalCancelled != null) await globalCancelled;
      if (cancelled != null) await cancelled;
      if (_disposed || !identical(agent, _agent)) return;
      _sub = agent.subscribe().listen(
        _onEvent,
        onError: (Object e) => AppLog.info('agent events: $e'),
      );
      if (id != null) {
        _uiSub = agent
            .subscribe(sessionId: id)
            .listen(
              _onUiEvent,
              onError: (Object e) => AppLog.info('agent page actions: $e'),
            );
      }
      // Replacing the local forwarders can span a run transition. Catch up
      // from the durable snapshot after subscribing so Working/Stop stay
      // accurate even if their initial event crossed that brief gap.
      await _loadState();
    } else {
      if (cancelled != null) await cancelled;
      if (id == null) return;
      final hub = hubLink;
      if (hub == null) return;
      // A separate viewing connection can close its session subscription
      // without disturbing the main connection or a running hub reply.
      final connection = await NotesAgent.connect(
        url: hub.url,
        device: vault.deviceId,
        token: hub.token,
        trusted: _hubTrust(hub.url),
      );
      if (_disposed || !identical(agent, _agent) || visibleSessionId != id) {
        await connection.closeEvents();
        connection.dispose();
        _scheduleViewScope();
        return;
      }
      _uiAgent = connection;
      _uiSub = connection
          .subscribe(sessionId: id)
          .listen(
            _onUiEvent,
            onError: (Object e) => AppLog.info('agent page actions: $e'),
          );
    }
  }

  void _onUiEvent(AgentEvent event) {
    final action = event.uiAction;
    if (!_disposed && action != null && action.sessionId == visibleSessionId) {
      _uiActions.add(action);
    }
  }

  @visibleForTesting
  void debugUiAction(AgentUiAction action) {
    if (!_disposed && action.sessionId == visibleSessionId) {
      _uiActions.add(action);
    }
  }

  @visibleForTesting
  void debugSetSnapshot(AgentSessionState snapshot) {
    openId = snapshot.session.id;
    state = snapshot;
    _notify();
    _scheduleViewScope();
  }

  @visibleForTesting
  void debugEvent(AgentEvent event) => _onEvent(event);

  @visibleForTesting
  void debugAddApproval(AgentApproval a) {
    _testApprovals.add(a);
    _notify();
  }

  /// Adds an assistant reply to the open chat's snapshot (fake state for
  /// widget tests: the fake provider only answers "ok"). The next reload
  /// drops it.
  @visibleForTesting
  void debugAddReply(String text) {
    final s = state;
    if (s == null) return;
    final reply = AgentMessage(
      id: 'test-reply-${s.messages.length}',
      role: 'assistant',
      content: text,
      text: text,
      toolCalls: const [],
      isError: false,
      createdAt: DateTime.now().toUtc().toIso8601String(),
    );
    state = AgentSessionState(
      session: s.session,
      messages: [...s.messages, reply],
      runs: s.runs,
      pendingApprovals: s.pendingApprovals,
      activity: s.activity,
      edits: s.edits,
      seq: s.seq,
      liveSeen: s.liveSeen,
      streamingText: s.streamingText,
    );
    _notify();
  }

  // ------------------------------------------------------------ opening

  String _configKey() {
    final hub = host == AgentHost.hub ? hubLink : null;
    return [
      enabled,
      host,
      hub?.url,
      hub?.token,
      // Trusting or revoking the hub's certificate reconnects with it.
      hub == null ? '' : _appSettings().trustedCertificates.trustKeyFor(hub.url),
      resolvedModel(AgentHost.local).model ?? '',
      settings.agentBunPath.trim(),
      settings.agentClaudeCodeDir.trim(),
      settings.desktopTools,
      settings.desktopRoot.trim(),
      vaultDefaults.toolSetFor(AgentHost.local),
      _dataDir,
    ].join('\u0000');
  }

  void _onSettings() => _settingsChanged();

  // ------------------------------- host relay

  /// "Let the hub use this computer" ([HostRelayLink]).
  late final _relay = HostRelayLink(onChanged: _notify);
  HostRelayLink get relay => _relay;

  /// Whether "Let the hub use this computer" is on and can run: it then
  /// shares this computer's tools and MCP servers, whichever runner the
  /// vault uses.
  bool get sharesThisComputer => _relayConfig() != null;

  /// The relay's settings while it should run: desktop, the toggle on and
  /// signed in to an https hub.
  HostRelayConfig? _relayConfig() {
    if (!localAgentPlatform || _disposed) return null;
    final app = _appSettings();
    if (!app.hubUsesThisComputer) return null;
    final hub = HubLink.of(app, allowLoopback: kDebugMode);
    if (hub == null) return null;
    final s = settings;
    String? opt(String v) => v.trim().isEmpty ? null : v.trim();
    final dataDir = '${_folder()}${Platform.pathSeparator}host-relay';
    final agentSettings = AgentSettings(
      defaultModel: '',
      bunPath: opt(s.agentBunPath),
      claudeCodeDir: opt(s.agentClaudeCodeDir),
      permissionMode: bridge.AgentPermissionMode.bypass,
      testTools: testTools,
      desktopTools: s.desktopTools,
      desktopRoot: s.desktopRoot.trim(),
    );
    return HostRelayConfig(
      url: hub.url,
      device: vault.deviceId,
      token: hub.token,
      trusted: _hubTrust(hub.url),
      trustKey: app.trustedCertificates.trustKeyFor(hub.url),
      open: () => NotesAgent.open(
        vault: vault.raw,
        dataDir: dataDir,
        settings: agentSettings,
      ),
      key: [
        hub.url,
        hub.token,
        vault.deviceId,
        dataDir,
        agentSettings.bunPath,
        agentSettings.claudeCodeDir,
        agentSettings.desktopTools,
        agentSettings.desktopRoot,
        testTools,
      ].join('\u0000'),
    );
  }

  String _hubConnectionKey() {
    final hub = hubLink;
    return '${hub?.url}\u0000${hub?.token}';
  }

  // ------------------------------- MCP servers (desktop)

  /// The AI settings' MCP servers in the shape `setMcpServers` takes.
  String get _mcpJson => jsonEncode(settings.agentMcpServers);

  /// What the open chat engine was last given (null: nothing yet).
  String? _mcpSent;

  /// Why the chat engine refused the MCP servers (null: applied). The
  /// relay engine's is [HostRelayLink.mcpError].
  String? mcpError;

  /// Tests: stand-ins for the engine's MCP calls and the one-time import.
  @visibleForTesting
  static Future<void> Function(String json)? debugSetMcpServers;
  @visibleForTesting
  static Future<List<AgentMcpServer>> Function()? debugMcpStatus;
  @visibleForTesting
  static Future<String?> Function(String dataDir)? debugMcpImport;

  /// Gives both desktop engines the current server list: they restart only
  /// the servers that changed, and the relay keeps its socket.
  void _syncMcp() {
    if (!localAgentPlatform || _disposed) return;
    final json = _mcpJson;
    _relay.setMcpServers(json);
    final a = _agent;
    if (a == null || _agentOnHub || json == _mcpSent) return;
    unawaited(_sendMcp(a, json));
  }

  Future<void> _sendMcp(NotesAgent a, String json) async {
    _mcpSent = json;
    String? failed;
    try {
      final debug = debugSetMcpServers;
      await _count(
        () => debug != null ? debug(json) : a.setMcpServers(json: json),
      );
    } catch (e) {
      failed = '$e';
      AppLog.info('agent MCP servers: $e');
    }
    if (!identical(a, _agent)) return;
    mcpError = failed;
    _notify();
  }

  /// The chat engine's MCP servers and their state; null while this
  /// computer's engine is not open (closed, or the tab is on the hub).
  Future<List<AgentMcpServer>?> mcpStatus() async {
    final a = _agent;
    if (a == null || _agentOnHub || _disposed) return null;
    try {
      final debug = debugMcpStatus;
      return await _count(() => debug != null ? debug() : a.mcpStatus());
    } catch (e) {
      AppLog.info('agent MCP status: $e');
      return null;
    }
  }

  Future<void>? _mcpImport;

  /// Imports `<data dir>/mcp.toml` into the AI settings once (desktop):
  /// servers whose names are taken are skipped. The bridge renames the file
  /// to `mcp.toml.imported`, so later starts find nothing to import.
  Future<void> importMcpFile() => _mcpImport ??= _importMcpFile();

  /// Tests: forget that this store already imported, like a new start.
  @visibleForTesting
  void debugForgetMcpImport() => _mcpImport = null;

  Future<void> _importMcpFile() async {
    if (!localAgentPlatform || _disposed) return;
    try {
      final dir = _folder();
      final debug = debugMcpImport;
      final json = debug != null
          ? await debug(dir)
          : await agentMcpImportFile(dataDir: dir);
      if (json == null || _disposed) return;
      final found = jsonDecode(json);
      if (found is! Map) return;
      final taken = settings.agentMcpServers;
      final added = <String, Map<String, dynamic>>{
        for (final e in found.entries)
          if (e.key is String &&
              e.value is Map &&
              !taken.containsKey(e.key))
            e.key as String: Map<String, dynamic>.from(e.value as Map),
      };
      AppLog.info(
        'mcp.toml imported: ${added.length} of ${found.length} servers added',
      );
      if (added.isEmpty) return;
      settings.update(
        (s) => s.agentMcpServers = {...s.agentMcpServers, ...added},
      );
    } catch (e) {
      AppLog.info('mcp.toml import failed: $e');
    }
  }

  void _settingsChanged({bool deferNotify = false}) {
    _syncMcp();
    _relay.sync(_relayConfig());
    // A new default (or a hub coming or going) brings the tab back to it.
    final def = defaultHost;
    if (def != _seenDefault) {
      _seenDefault = def;
      _hostOverride = null;
    }
    final key = _configKey();
    final changed = key != _key;
    final hubKey = _hubConnectionKey();
    if (hubAgentPlatform && hubKey != _hubKey) {
      // A new login or Hub must never reuse another host's chat IDs or cache.
      _resetHubState = true;
      sessions = const [];
      models = const [];
      openId = null;
      state = null;
      streaming = '';
      error = null;
      openError = null;
      _live.clear();
      _testApprovals.clear();
      _modelBySession.clear();
      _pendingAnchor = null;
      _clearOutline();
      _stopOutlinePoll();
    }
    _hubKey = hubKey;
    if (changed) hostInfo = null;
    if (deferNotify) {
      scheduleMicrotask(_notify);
    } else {
      _notify();
    }
    if (!changed) return;
    _key = key;
    if (_agent == null && _opening == null) {
      // An explicit first open already uses the latest settings. A queued
      // reopen here would close that new agent after the debounce expires.
      _reopen.cancel();
      _resetHubState = false;
      return;
    }
    // Text settings change per keystroke: reopen once they rest.
    _reopen.run(const Duration(milliseconds: 600), () async {
      if (_disposed) return;
      final keepState = !_resetHubState;
      await _close(keepState: keepState);
      _resetHubState = false;
      if (enabled) await ensureOpen();
      _notify();
    });
  }

  String _folder() {
    final d = _dataDir;
    if (d != null) return d;
    final f = settings.file;
    if (f != null) return f.parent.path;
    // The default vault lives in `<data dir>/vault`.
    return Directory(vault.root).parent.path;
  }

  /// Opens the agent if it is on and not open yet. Null when it is off or
  /// failed to open ([openError] says why).
  Future<NotesAgent?> ensureOpen() {
    if (!enabled || _disposed || requiresHubSignIn || _resetHubState) {
      return Future.value(null);
    }
    if (_agent != null) return Future.value(_agent);
    return _opening ??= _open().whenComplete(() => _opening = null);
  }

  /// An agent on [hub], or on this computer when it is null.
  Future<NotesAgent> _connect(HubLink? hub) {
    final s = settings;
    String? opt(String v) => v.trim().isEmpty ? null : v.trim();
    return hub != null
        ? NotesAgent.connect(
            url: hub.url,
            device: vault.deviceId,
            token: hub.token,
            trusted: _hubTrust(hub.url),
          )
        : NotesAgent.open(
            vault: vault.raw,
            dataDir: _folder(),
            settings: AgentSettings(
              defaultModel: resolvedModel(AgentHost.local).model ?? '',
              bunPath: opt(s.agentBunPath),
              claudeCodeDir: opt(s.agentClaudeCodeDir),
              permissionMode: bridge.AgentPermissionMode.bypass,
              testTools: testTools,
              desktopTools: s.desktopTools,
              desktopRoot: s.desktopRoot.trim(),
            ),
          );
  }

  Future<NotesAgent?> _open() async {
    // Android never attempts the desktop engine, even with stale preferences.
    if (requiresHubSignIn) return null;
    final openingKey = _configKey();
    final hub = host == AgentHost.hub ? hubLink : null;
    try {
      // Settings may be reading this computer's models with a short-lived
      // engine: let it close before this one opens.
      if (hub == null) await _localModelsRead;
      final agent = await _count(() => _connect(hub));
      if (_disposed || openingKey != _configKey()) {
        await agent.closeEvents();
        agent.dispose();
        return null;
      }
      final AgentHostInfo info;
      try {
        info = await _count(agent.hostInfo);
      } catch (_) {
        agent.dispose();
        rethrow;
      }
      if (_disposed || openingKey != _configKey()) {
        await agent.closeEvents();
        agent.dispose();
        return null;
      }
      _agent = agent;
      _agentOnHub = hub != null;
      _search ??= _newSearchIndex();
      _syncOutlinePoll();
      _syncSearchPoll();
      openError = null;
      hostInfo = info;
      if (hub == null) {
        claudeDetection = await _count(agent.claudeDetection);
        // A new engine starts with no servers: give it the list.
        _mcpSent = null;
        await _sendMcp(agent, _mcpJson);
      }
      _sub = agent.subscribe().listen(
        _onEvent,
        onError: (Object e) => AppLog.info('agent events: $e'),
      );
      _subscribedSession = null;
      _scheduleViewScope();
      await refreshProjects();
      await _loadModels();
      await refreshSessions();
      if (openId != null) await _loadState();
      return agent;
    } catch (e) {
      openError = '$e';
      AppLog.info('agent failed to open: $e');
      _notify();
      return null;
    }
  }

  Future<void> _loadModels() async {
    final a = _agent;
    if (a == null) return;
    final on = _agentOnHub ? AgentHost.hub : AgentHost.local;
    try {
      models = _withExtras(await _count(a.models), hostInfo);
      _modelsFailed = false;
    } catch (e) {
      models = const [];
      _modelsFailed = true;
      AppLog.info('agent models: $e');
    }
    _hostModels[on] = _HostModels(
      _modelsKey(on),
      models,
      failed: _modelsFailed,
      hostDefault: hostInfo?.defaultModel,
    );
    if (on == AgentHost.hub && !_modelsFailed) {
      _rememberHubModels(models, hostInfo?.defaultModel);
    }
    _notify();
  }

  /// Dormant permission endpoint retained for future mode controls. The
  /// shipped bridge keeps Bypass and leaves the saved preference untouched.
  Future<void> setPermissionMode(AgentPermissionMode mode) async {
    final agent = await ensureOpen();
    if (agent == null) return;
    try {
      await _count(
        () => agent.permissionModeSet(
          mode: bridge.AgentPermissionMode.values.byName(mode.name),
        ),
      );
      hostInfo = await _count(agent.hostInfo);
    } catch (e) {
      error = '$e';
    }
    _notify();
  }

  Future<void> refreshProjects() async {
    final a = _agent;
    if (a == null || !(hostInfo?.projects ?? false)) {
      projects = const [];
      return;
    }
    try {
      final loaded = await _count(a.projectsList);
      if (!identical(a, _agent) || _disposed) return;
      projects = loaded;
    } catch (e) {
      error = '$e';
    }
    _notify();
  }

  Future<void> addProject(String name, String path) async {
    final a = await ensureOpen();
    if (a == null) return;
    try {
      await _count(() => a.projectAdd(name: name.trim(), path: path.trim()));
      error = null;
      await refreshProjects();
    } catch (e) {
      error = '$e';
      _notify();
    }
  }

  Future<void> renameProject(String id, String name) async {
    final a = await ensureOpen();
    if (a == null || name.trim().isEmpty) return;
    try {
      await _count(() => a.projectRename(id: id, name: name.trim()));
      error = null;
      await refreshProjects();
      await refreshSessions();
      await _loadState();
    } catch (e) {
      error = '$e';
      _notify();
    }
  }

  Future<void> removeProject(String id) async {
    final a = await ensureOpen();
    if (a == null || projects.length <= 1) return;
    try {
      await _count(() => a.projectRemove(id: id));
      error = null;
      await refreshProjects();
      await refreshSessions();
      await _loadState();
    } catch (e) {
      error = '$e';
      _notify();
    }
  }

  Future<void> setSessionProject(String sessionId, String projectId) async {
    final a = await ensureOpen();
    if (a == null) return;
    try {
      await _count(
        () => a.sessionProjectSet(sessionId: sessionId, projectId: projectId),
      );
      error = null;
      await refreshSessions();
      if (openId == sessionId) await _loadState();
    } catch (e) {
      error = '$e';
      _notify();
    }
  }

  // ------------------------------------------------------------ chats

  Future<void> refreshSessions() async {
    final a = _agent;
    if (a == null) return;
    try {
      sessions = await _count(a.sessions);
      _userNamed = (await _count(a.sessionUserNamedIds)).toSet();
    } catch (e) {
      AppLog.info('agent sessions: $e');
    }
    _syncSearch();
    _notify();
  }

  /// Installs a fixture transcript and outline for the open chat and notifies
  /// it. Tests only. The view subscription reloads the database's session when
  /// a chat mounts, so a fixture must come after the mount has settled.
  @visibleForTesting
  void seedFixture(AgentSessionState snapshot, ChatOutline chapters) {
    final id = snapshot.session.id;
    state = snapshot;
    outline = chapters;
    outlineFor = id;
    _notify();
  }

  void setFilter(String q) {
    filter = q;
    _notify();
  }

  /// Starts a chat and opens it.
  Future<AgentSession?> createSession({String title = 'New chat'}) async {
    final a = await ensureOpen();
    if (a == null) return null;
    final s = await _count(() => a.createSession(title: title));
    final pending = _modelBySession.remove('');
    if (pending != null) _modelBySession[s.id] = pending;
    _prefs.created(s.id, host, model: pending);
    _created.add(s.id);
    await refreshSessions();
    await openSession(s.id);
    return s;
  }

  Future<void> renameSession(String id, String title) async {
    final a = await ensureOpen();
    final t = title.trim();
    if (a == null || t.isEmpty) return;
    try {
      await _count(() => a.renameSession(sessionId: id, title: t));
    } catch (e) {
      error = '$e';
    }
    await refreshSessions();
    if (openId == id) await _loadState();
  }

  Future<void> deleteSession(String id) async {
    final a = await ensureOpen();
    if (a == null) return;
    try {
      await _count(() => a.deleteSession(sessionId: id));
    } catch (e) {
      error = '$e';
      _notify();
      return;
    }
    _search?.removeSession(id);
    _modelBySession.remove(id);
    _prefs.remove(id);
    _created.remove(id);
    if (openId == id) {
      openId = null;
      state = null;
      streaming = '';
      _live.clear();
      _testApprovals.clear();
      _clearOutline();
    }
    if (_pendingAnchor?.sessionId == id) _pendingAnchor = null;
    await refreshSessions();
  }

  /// Forgets the open chat's outline and any load still in flight for it.
  void _clearOutline() {
    outline = ChatOutline(chapters: const []);
    outlineFor = null;
    _outlineKey = null;
    _outlineRevision = null;
    _outlineLoads++;
  }

  AgentSearchIndex? _newSearchIndex() {
    final sinkFor = searchSinkFor;
    return sinkFor == null ? null : AgentSearchIndex(sinkFor(vault));
  }

  /// Sweeps the chats into the search index when an agent is open. With no
  /// agent (disabled, offline or closed) the index keeps its cached rows.
  void _syncSearch() {
    final a = _agent;
    final search = _search;
    if (a == null || search == null || _disposed) return;
    unawaited(
      search.sync(
        _AgentSearchReader(a),
        outlines: !_agentOnHub,
        chatTitle: _searchTitle,
      ),
    );
  }

  void _syncSearchPoll() {
    if (_disposed || _agent == null || _search == null) {
      _stopSearchPoll();
      return;
    }
    _searchPoll ??= Timer.periodic(searchSweepInterval, (_) => _syncSearch());
  }

  void _stopSearchPoll() {
    _searchPoll?.cancel();
    _searchPoll = null;
  }

  /// Polls the outline revision while a chat is on screen and the agent is
  /// open. One poll runs at a time; the timer stops when neither holds.
  void _syncOutlinePoll() {
    final live = !_disposed && _agent != null && visibleSessionId != null;
    if (!live) {
      _stopOutlinePoll();
      return;
    }
    _outlinePoll ??= Timer.periodic(
      outlinePollInterval,
      (_) => unawaited(_pollOutline()),
    );
  }

  void _stopOutlinePoll() {
    _outlinePoll?.cancel();
    _outlinePoll = null;
  }

  Future<void> _pollOutline() async {
    final a = _agent, id = visibleSessionId;
    if (_outlinePolling || a == null || id == null) return;
    if (outlineFor != id || openId != id) return;
    _outlinePolling = true;
    try {
      final rev = await _count(
        () => (debugRevisionLoader ?? _defaultRevision)(a, id),
      );
      if (_disposed || !identical(a, _agent) || openId != id) return;
      if (visibleSessionId != id) return;
      // The first revision read after a fixture or a failed load is the
      // baseline; only a later change reloads.
      final before = _outlineRevision;
      if (before == null) {
        _outlineRevision = rev;
      } else if (rev != before) {
        await _loadOutline(a, id, _outlineKey ?? '', revision: rev);
        await _refreshMetadata(a, id);
      }
    } catch (e) {
      // A missing chat or a failed read ends polling; the next load of a
      // visible chat starts it again.
      AppLog.info('agent outline revision: $e');
      _stopOutlinePoll();
    } finally {
      _outlinePolling = false;
    }
  }

  /// Reloads the open chat's metadata and the chat list after a revision
  /// change, so an automatic title change shows without a reload of the chat.
  Future<void> _refreshMetadata(NotesAgent a, String id) async {
    try {
      final s = await _count(() => a.sessionState(sessionId: id));
      if (_disposed || !identical(a, _agent) || openId != id) return;
      final current = state;
      if (current != null && current.session.id == id) {
        state = AgentSessionState(
          session: s.session,
          messages: current.messages,
          runs: current.runs,
          pendingApprovals: current.pendingApprovals,
          activity: current.activity,
          edits: current.edits,
          seq: current.seq,
          liveSeen: current.liveSeen,
          streamingText: current.streamingText,
        );
        _notify();
      }
    } catch (e) {
      AppLog.info('agent session refresh: $e');
    }
    unawaited(refreshSessions());
  }

  /// Opens chat [id] (null: none).
  /// Opens [sessionId] and asks the open chat to show the span from
  /// [firstMessageId] to [lastMessageId] once the chat has loaded. [presentation]
  /// names the presentation that should show it; without one, the first chat
  /// that shows the span takes it.
  Future<void> openReference(
    String sessionId,
    String firstMessageId,
    String lastMessageId, {
    AgentPresentation? presentation,
  }) async {
    _pendingAnchor = AgentAnchor(
      sessionId: sessionId,
      firstMessageId: firstMessageId,
      lastMessageId: lastMessageId,
      presentation: presentation,
    );
    await openSession(sessionId);
    _notify();
  }

  Future<void> openSession(String? id) async {
    final saved = id == null ? null : hostOf(id);
    if (saved != null && saved != host && usableHosts.contains(saved)) {
      // Switching closes this computer's agent, which stops its reply:
      // never without asking.
      if (running && host == AgentHost.local) {
        final ask = _switchConfirms.lastOrNull;
        if (ask == null || !await ask(host)) return;
        if (_disposed) return;
      }
      await _switchHost(saved);
    }
    if (openId == id && state != null) return;
    if (_pendingAnchor?.sessionId != id) _pendingAnchor = null;
    openId = id;
    _clearOutline();
    _syncOutlinePoll();
    _liveActivitySummary = null;
    _scheduleViewScope();
    state = null;
    streaming = '';
    error = null;
    _live.clear();
    _testApprovals.clear();
    _snapSeq = _seenSeq = 0;
    _notify();
    if (id != null) await _loadState();
  }

  Future<void> _loadState() async {
    final a = _agent, id = openId;
    if (a == null || id == null) return;
    if (_loading) {
      _reloadAgain = true;
      return;
    }
    _loading = true;
    try {
      final s = await _count(() => a.sessionState(sessionId: id));
      if (openId != id || _disposed) return;
      _applySnapshot(s);
      final key = '${s.messages.length}:${s.messages.lastOrNull?.id}';
      if (outlineFor != id || _outlineKey != key) {
        await _loadOutline(a, id, key);
      }
    } catch (e) {
      if (openId == id) error = '$e';
    } finally {
      _loading = false;
      _notify();
    }
    if (_reloadAgain) {
      _reloadAgain = false;
      await _loadState();
    }
  }

  /// Loads the outline of [id] for the message list [key]. The revision is
  /// read first, so a change that lands during the load shows on the next
  /// poll. A load superseded by a newer one, by another chat or by another
  /// agent is dropped. A failed load leaves the chat without chapters: the
  /// plain transcript.
  Future<void> _loadOutline(
    NotesAgent a,
    String id,
    String key, {
    BigInt? revision,
  }) async {
    final gen = ++_outlineLoads;
    bool current() =>
        gen == _outlineLoads &&
        openId == id &&
        identical(a, _agent) &&
        !_disposed;
    final rev = revision ?? await _revisionOrNull(a, id);
    if (!current()) return;
    ChatOutline next;
    try {
      next = await _count(() => a.sessionOutline(sessionId: id));
    } catch (_) {
      next = ChatOutline(chapters: const []);
    }
    if (!current()) return;
    outline = next;
    outlineFor = id;
    _outlineKey = key;
    _outlineRevision = rev;
    _syncSearch();
    _notify();
  }

  Future<BigInt?> _revisionOrNull(NotesAgent a, String id) async {
    try {
      return await _count(
        () => (debugRevisionLoader ?? _defaultRevision)(a, id),
      );
    } catch (_) {
      return null;
    }
  }

  void _applySnapshot(AgentSessionState s) {
    state = s;
    _liveActivitySummary = null;
    if (hostInfo?.projects ?? false) unawaited(refreshProjects());
    final seq = s.seq.toInt();
    _snapSeq = seq;
    if (seq > _seenSeq) _seenSeq = seq;
    _live.removeWhere((e) => e.sequence.toInt() < seq);
    _compose();
  }

  /// The reply being written: the snapshot's text, then the live paragraphs
  /// it does not hold yet (its first `live_seen` at its `seq` are in it).
  void _compose() {
    final s = state;
    if (s == null) return;
    var skip = s.liveSeen.toInt();
    final parts = <String>[if (s.streamingText.isNotEmpty) s.streamingText];
    for (final e in _live) {
      if (e.sequence.toInt() == _snapSeq && skip > 0) {
        skip--;
        continue;
      }
      parts.add(e.text ?? '');
    }
    streaming = parts.join(_paragraphGap);
  }

  // ------------------------------------------------------------ events

  void _onEvent(AgentEvent e) {
    if (_disposed) return;
    if (e.kind == 'gap') {
      unawaited(_catchUp(e.sequence.toInt()));
      return;
    }
    if (e.kind == 'run_started' || e.kind == 'run_finished') {
      _reloadList.run(const Duration(milliseconds: 100), refreshSessions);
    }
    if (e.sessionId != openId) return;
    if (e.kind == 'tool_started') {
      _liveActivitySummary = switch (e.toolName ?? e.toolCall?.name) {
        'search_notes' => 'Searching notes',
        'search' => 'Searching files',
        'read_page' => 'Reading a page',
        'read' => 'Reading a file',
        'write' || 'edit' => 'Editing a file',
        'list' => 'Listing files',
        'exec' => 'Running a command',
        'edit_page' => 'Editing a page',
        'js' => _liveActivitySummary,
        final String name => name,
        _ => _liveActivitySummary,
      };
      _notify();
    }
    if (e.kind == 'run_finished') {
      _liveActivitySummary = null;
      if (hubAgentPlatform) _appState?.sync.syncNow();
    }
    if (e.kind == 'tool_started' || e.kind == 'tool_finished') {
      // Nested tool progress is ephemeral, but Part A records its activity
      // and edits in SQLite. Refresh it while the reply is still running.
      _reload.run(const Duration(milliseconds: 30), _loadState);
    }
    if (!e.durable) {
      if (e.kind != 'paragraph') return;
      paragraphsSeen++;
      if (e.sequence.toInt() < _snapSeq) return;
      _live.add(e);
      final text = e.text ?? '';
      // A paragraph at the snapshot's sequence may already be in it; the
      // next snapshot sorts that out, so show it now only past the skip.
      streaming = streaming.isEmpty ? text : '$streaming\n\n$text';
      _notify();
      return;
    }
    if (e.sequence.toInt() <= _seenSeq) return;
    _seenSeq = e.sequence.toInt();
    if (e.kind == 'run_finished' && (e.error ?? '').isNotEmpty) {
      error = null;
    }
    _reload.run(const Duration(milliseconds: 30), _loadState);
  }

  Future<void> _catchUp(int after) async {
    final a = _agent;
    if (a == null) return;
    try {
      final page = await _count(
        () => a.eventsAfter(
          after: PlatformInt64Util.from(after),
          sessionId: openId,
        ),
      );
      if (page.resync) {
        await _loadState();
      } else {
        for (final e in page.events) {
          _onEvent(e);
        }
      }
    } catch (e) {
      AppLog.info('agent catch-up: $e');
    }
    await refreshSessions();
  }

  // ------------------------------------------------------------ runs

  /// Sends [text] in the open chat: a new run, or steering while one runs.
  Future<void> send(
    String text, {
    AgentMessageContext? context,
    String? sessionId,
  }) async {
    final id = sessionId ?? openId;
    final t = text.trim();
    if (id == null || t.isEmpty) return;
    final a = await ensureOpen();
    if (a == null) return;
    error = null;
    try {
      // On Android the tools read the Hub replica. Finish local saves and a
      // sync round before asking the model to read or edit notes.
      if (hubAgentPlatform) {
        final app = _appState;
        if (app == null ||
            !app.sync.connected ||
            app.sync.hubUrl != hubLink?.url) {
          throw StateError(
            '$appName is not syncing with the Hub. Sign in again in Settings > AI.',
          );
        }
        await app.pageStore.lastSave;
        await vault.flush();
        final synced = await app.sync.syncOnce();
        if (_disposed ||
            requiresHubSignIn ||
            _resetHubState ||
            !identical(a, _agent)) {
          return;
        }
        if (synced == null || !app.sync.connected) {
          throw StateError('$appName disconnected from the Hub before the turn.');
        }
      }
      // send queues a follow-up while busy and preserves its structured
      // attachments; the older steer endpoint accepts text only.
      await _count(
        () => a.send(
          sessionId: id,
          text: replay?.sessionId == id ? text : t,
          model: modelOf(id),
          context: context,
        ),
      );
    } catch (e) {
      error = '$e';
    }
    await _loadState();
  }

  Future<AgentEdit> undoEdit(String editId) async {
    final agent = await ensureOpen();
    if (agent == null) {
      throw StateError(openError ?? 'The agent is not available.');
    }
    final edit = await _count(() => agent.editUndo(editId: editId));
    await _loadState();
    return edit;
  }

  /// Stops the open chat's run.
  Future<void> stop() async {
    final id = openId, a = _agent;
    if (id == null || a == null) return;
    try {
      await _count(() => a.cancel(sessionId: id));
    } catch (e) {
      error = '$e';
    }
    await _loadState();
  }

  /// [decision] is `approve`, `approve_session` or `deny`.
  Future<void> decide(
    String approvalId,
    String decision, {
    String? reason,
  }) async {
    final test = _testApprovals.indexWhere((x) => x.id == approvalId);
    if (test >= 0) {
      _testApprovals.removeAt(test);
      _notify();
      return;
    }
    final a = _agent;
    if (a == null) return;
    try {
      await _count(
        () => a.decideApproval(
          approvalId: approvalId,
          decision: decision,
          reason: reason,
        ),
      );
    } catch (e) {
      error = '$e';
    }
    await _loadState();
  }

  /// Runs [code] as a `js` cell in chat [id] without a model.
  Future<AgentRunRecord?> runScript(
    String id,
    String label,
    String code,
  ) async {
    final a = await ensureOpen();
    if (a == null) return null;
    final r = await _count(
      () => a.runScript(sessionId: id, label: label, code: code),
    );
    if (openId == id) await _loadState();
    return r;
  }

  // ------------------------------------------------------------ closing

  static Future<T> _count<T>(Future<T> Function() f) async {
    Vault.inFlight.value++;
    try {
      return await f();
    } finally {
      Vault.inFlight.value--;
    }
  }

  Future<void> _close({bool keepState = false, bool cancelOpen = true}) async {
    final playback = replay;
    replay = null;
    if (playback != null) {
      playback.removeListener(_notify);
      try {
        await playback.stop();
      } catch (_) {}
      playback.dispose();
    }
    _stopOutlinePoll();
    _stopSearchPoll();
    _viewScope.cancel();
    await _scopeWork;
    final viewer = _uiAgent;
    _uiAgent = null;
    final uiCancelled = _uiSub?.cancel();
    _uiSub = null;
    if (viewer != null) {
      await viewer.closeEvents();
      viewer.dispose();
    }
    _subscribedSession = null;
    final opening = _opening;
    if (opening != null) await opening;
    final a = _agent;
    _agent = null;
    final sub = _sub;
    _sub = null;
    // Cancelling waits until the stream yields again: closing it on the
    // Rust side ends it now. The timeout only guards against a lost close.
    final cancelled = sub?.cancel();
    if (a != null) {
      try {
        await a.closeEvents();
      } catch (e) {
        AppLog.info('agent close events: $e');
      }
    }
    if (cancelled != null) {
      await cancelled.timeout(
        closeTimeout,
        onTimeout: () => AppLog.info('agent events: close timed out'),
      );
    }
    if (uiCancelled != null) {
      await uiCancelled.timeout(closeTimeout, onTimeout: () {});
    }
    if (a != null) {
      final id = openId;
      if (id != null && cancelOpen) {
        try {
          await a.cancel(sessionId: id);
        } catch (_) {}
      }
      a.dispose();
    }
    models = const [];
    hostInfo = null;
    claudeDetection = null;
    projects = const [];
    if (!keepState) {
      sessions = const [];
      state = null;
      openId = null;
      _clearOutline();
      _pendingAnchor = null;
    }
  }

  /// How long closing waits for the event stream to end.
  static const closeTimeout = Duration(seconds: 2);

  /// Closes the agent and disposes the store. The search index drains first,
  /// so no row write is still running when the caller closes the vault.
  Future<void> close() async {
    if (_disposed) return;
    _reopen.cancel();
    _viewScope.cancel();
    _reload.cancel();
    _reloadList.cancel();
    _stopSearchPoll();
    await _search?.close();
    await _close();
    dispose();
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _reopen.cancel();
    _reload.cancel();
    _reloadList.cancel();
    _stopOutlinePoll();
    _stopSearchPoll();
    // Stops new index work now; queued deletes still drain in the background.
    unawaited(_search?.close());
    _settings.removeListener(_onSettings);
    _app?.removeListener(_onSettings);
    unawaited(_relay.close());
    if (_agent != null || _sub != null) unawaited(_close());
    unawaited(_uiActions.close());
    _stores[vault] = null;
    super.dispose();
  }
}

/// Reads the search index's inputs from an open agent.
class _AgentSearchReader implements AgentSearchReader {
  _AgentSearchReader(this._agent);

  final NotesAgent _agent;

  @override
  Future<List<AgentSession>> sessions() => _agent.sessions();

  @override
  Future<BigInt> revision(String sessionId) =>
      _agent.sessionOutlineRevision(sessionId: sessionId);

  @override
  Future<ChatOutline> outline(String sessionId) =>
      _agent.sessionOutline(sessionId: sessionId);
}

/// A host's model list as Settings last read it.
class _HostModels {
  const _HostModels(
    this.key,
    this.models, {
    required this.failed,
    this.hostDefault,
  });
  final String key;
  final List<AgentModel> models;
  final bool failed;

  /// The host's own default model, when it reports one.
  final String? hostDefault;
}

/// A runner's models for Settings ([AgentStore.runnerModels]).
class RunnerModels {
  const RunnerModels({
    this.models = const [],
    this.hostDefault,
    this.live = false,
    this.unreachable = false,
  });

  final List<AgentModel> models;

  /// The runner's own default model (the hub's `[agent] default_model`).
  final String? hostDefault;

  /// Read from the runner now (else the last known list, or none yet).
  final bool live;

  /// The runner can't be reached; [models] is the last known list.
  final bool unreachable;
}
