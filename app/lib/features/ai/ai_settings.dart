// AI settings of this device: the in-app agent (on or off, this device's
// override of where it runs and of the default model, Claude Code extension,
// approvals). Kept in `<data dir>/ai_settings.json` next to settings.json
// (per device, never synced). The default runner and default model themselves
// are vault settings, synced (see [resolveAgentHost]); this file only holds
// the device's overrides. The only AI that runs inside the app is the agent
// (BubblePaw), here or on the hub; Claude Code, Codex and other apps
// connect to the vault from outside (the hub, agent tokens). Keys of
// the Ask AI runners removed on 2026-10-22 (the runner choice, the Claude
// Code and Codex program paths, the DeepSeek key, model and address, the
// run timeout) are ignored when loading and dropped on the next save.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../../core/tool_sets.dart';

/// How the agent handles tools that ask for confirmation.
enum AgentPermissionMode {
  manual(
    'Manual',
    'Asks before editing files, running commands or changing project.',
  ),
  auto(
    'Auto',
    'Works freely inside the current project; asks to open other folders.',
  ),
  bypass(
    'Bypass',
    'Never asks. The agent can change any file and run any command.',
  );

  const AgentPermissionMode(this.label, this.help);
  final String label;
  final String help;
}

/// Where the in-app agent runs.
enum AgentHost {
  local('This computer', 'this_device'),
  hub('Hub', 'hub');

  const AgentHost(this.label, this.runner);
  final String label;

  /// The runner name in the vault's synced agent defaults.
  final String runner;

  /// The host a vault runner name means on this device (null: unknown).
  static AgentHost? ofRunner(String? runner) =>
      values.where((h) => h.runner == runner).firstOrNull;
}

/// The hub's models as last read on this device, for when it can't be
/// reached (Settings > AI with Run AI on: Hub).
class HubModelList {
  const HubModelList({required this.models, this.defaultModel});

  final List<({String id, String name})> models;

  /// The hub's own default (`[agent] default_model`), when it reported it.
  final String? defaultModel;

  static HubModelList? fromJson(Object? j) {
    if (j is! Map) return null;
    final models = [
      for (final m in (j['models'] as List?) ?? const [])
        if (m is Map && m['id'] is String)
          (id: m['id'] as String, name: (m['name'] as String?) ?? m['id'] as String),
    ];
    if (models.isEmpty) return null;
    final d = j['default'];
    return HubModelList(models: models, defaultModel: d is String ? d : null);
  }

  Map<String, dynamic> toJson() => {
    'models': [
      for (final m in models) {'id': m.id, 'name': m.name},
    ],
    if (defaultModel != null) 'default': defaultModel,
  };

  @override
  bool operator ==(Object other) =>
      other is HubModelList &&
      other.defaultModel == defaultModel &&
      listEquals(other.models, models);

  @override
  int get hashCode => Object.hash(defaultModel, Object.hashAll(models));
}

/// Where an effective agent default came from.
enum AgentDefaultSource { device, vault, builtIn }

/// The vault's synced agent defaults, as this device reads them.
class VaultAgentDefaultsView {
  const VaultAgentDefaultsView({
    this.host,
    this.models = const {},
    this.toolSets = const {},
  });
  static const none = VaultAgentDefaultsView();

  /// The default runner (null: not set in the vault).
  final AgentHost? host;

  /// Default `provider/model` per runner.
  final Map<AgentHost, String> models;

  /// Tool set per runner (`read`, `write`, `everything`); a runner not
  /// listed has the default, [defaultAgentToolSet].
  final Map<AgentHost, String> toolSets;

  /// The tool set [runner]'s agent gets.
  String toolSetFor(AgentHost runner) =>
      toolSets[runner] ?? defaultAgentToolSet;

  @override
  bool operator ==(Object other) =>
      other is VaultAgentDefaultsView &&
      other.host == host &&
      mapEquals(other.models, models) &&
      mapEquals(other.toolSets, toolSets);

  @override
  int get hashCode => Object.hash(
    host,
    Object.hashAllUnordered(
      models.entries.map((e) => Object.hash(e.key, e.value)),
    ),
    Object.hashAllUnordered(
      toolSets.entries.map((e) => Object.hash(e.key, e.value)),
    ),
  );
}

/// The default runner on this device and why.
class ResolvedAgentHost {
  const ResolvedAgentHost(this.host, this.wanted, this.source);

  /// Where new chats run here.
  final AgentHost host;

  /// What the device override, the vault or the built-in default asks for.
  final AgentHost wanted;
  final AgentDefaultSource source;

  /// The wanted runner isn't reachable here, so [host] is used instead.
  bool get fellBack => host != wanted;
}

/// The built-in default runner: this computer, or the Hub on Android.
AgentHost get builtInAgentHost =>
    hubAgentPlatform ? AgentHost.hub : AgentHost.local;

/// Precedence (integration spec, phase 3): this device's override, then the
/// vault's value, then the built-in default; then, when that runner isn't in
/// [usable] (reachable here), the built-in default or the first usable one.
ResolvedAgentHost resolveAgentHost({
  AgentHost? device,
  AgentHost? vault,
  AgentHost? builtIn,
  required List<AgentHost> usable,
}) {
  final fallback = builtIn ?? builtInAgentHost;
  final (wanted, source) = device != null
      ? (device, AgentDefaultSource.device)
      : vault != null
      ? (vault, AgentDefaultSource.vault)
      : (fallback, AgentDefaultSource.builtIn);
  final host = usable.contains(wanted)
      ? wanted
      : usable.contains(fallback) || usable.isEmpty
      ? fallback
      : usable.first;
  return ResolvedAgentHost(host, wanted, source);
}

/// The default model for [runner]: this device's override, then the vault's
/// value (null model: none set; the host's own default applies).
({String? model, AgentDefaultSource? source}) resolveAgentModel(
  AgentHost runner, {
  Map<AgentHost, String> device = const {},
  Map<AgentHost, String> vault = const {},
}) {
  String? clean(String? m) => (m == null || m.trim().isEmpty) ? null : m.trim();
  final d = clean(device[runner]);
  if (d != null) return (model: d, source: AgentDefaultSource.device);
  final v = clean(vault[runner]);
  if (v != null) return (model: v, source: AgentDefaultSource.vault);
  return (model: null, source: null);
}

/// Whether this platform runs the local agent engine.
bool get localAgentPlatform =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.windows ||
        defaultTargetPlatform == TargetPlatform.linux);

/// Android chats always use the Hub; the local engine stays desktop-only.
bool get hubAgentPlatform =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.android;

bool get agentPlatform => localAgentPlatform || hubAgentPlatform;

class AiSettings extends ChangeNotifier {
  AiSettings({this.file});

  /// Where the settings are saved (null: in memory only).
  final File? file;

  /// The in-app agent (local on desktop, always available through the Hub
  /// on Android, including installs whose old settings saved it as off).
  bool agentEnabled = agentPlatform;

  /// This device's override of the vault's default model, per runner, as
  /// saved by older builds. No longer applied or shown (2026-10-22): the
  /// vault's setting decides.
  Map<AgentHost, String> agentModelOverrides = {};

  /// The hub's models as last read here ([HubModelList]).
  HubModelList? hubModels;

  /// Values saved by builds before the synced defaults (`agentHost`,
  /// `agentModel`). Kept until [AgentStore] copies them into the vault once
  /// (only when the vault has none) and clears them; never an override.
  AgentHost? legacyAgentHost;
  String? legacyAgentModel;
  bool get hasLegacyAgentDefaults =>
      legacyAgentHost != null || legacyAgentModel != null;

  /// Model visibility and extra provider/model ids, saved by older builds.
  /// No longer applied or shown (2026-10-22): pickers list what the runner
  /// reports.
  Set<String> agentHiddenModels = {};
  List<String> agentCustomModels = [];

  /// The Claude Code extension: the `bun` program and the extension folder
  /// (BubblePaw's `extensions/claude-code`). Empty: detected automatically.
  String agentBunPath = '';
  String agentClaudeCodeDir = '';

  /// Preserved preference for future controls; the shipped app uses Bypass.
  AgentPermissionMode agentPermissionMode = AgentPermissionMode.auto;

  /// Files and commands available to the agent on this computer.
  bool desktopTools = true;
  String desktopRoot =
      Platform.environment[Platform.isWindows ? 'USERPROFILE' : 'HOME'] ?? '';

  /// MCP servers of the desktop agent (in-app chats and the host relay), by
  /// name: each is BubblePaw's `McpServerSettings` as JSON (snake_case keys,
  /// e.g. `command`, `args`, `env` or `url`, `headers_from`, `enabled`).
  /// Keys the settings UI does not edit are kept as they are.
  Map<String, Map<String, dynamic>> agentMcpServers = {};

  /// The settings beside [settingsFile] (the app's settings.json).
  static AiSettings beside(File? settingsFile) {
    if (settingsFile == null) return AiSettings();
    return load(
      File(
        '${settingsFile.parent.path}${Platform.pathSeparator}ai_settings.json',
      ),
    );
  }

  /// Loads [file]; missing or unreadable settings give the defaults.
  static AiSettings load(File file) {
    final s = AiSettings(file: file);
    try {
      if (file.existsSync()) {
        s._fromJson(
          jsonDecode(file.readAsStringSync()) as Map<String, dynamic>,
        );
      }
    } catch (_) {}
    return s;
  }

  void _fromJson(Map<String, dynamic> j) {
    agentEnabled =
        hubAgentPlatform ||
        (localAgentPlatform && ((j['agentEnabled'] as bool?) ?? true));
    agentModelOverrides = {
      for (final e
          in ((j['agentModelOverrides'] as Map?) ?? const {}).entries)
        if (AgentHost.ofRunner(e.key as String?) case final h?)
          if (e.value is String && (e.value as String).trim().isNotEmpty)
            h: (e.value as String).trim(),
    };
    final legacyModel = j['agentModel'];
    legacyAgentModel = legacyModel is String && legacyModel.trim().isNotEmpty
        ? legacyModel.trim()
        : null;
    hubModels = HubModelList.fromJson(j['hubModels']);
    agentHiddenModels =
        (j['agentHiddenModels'] as List?)?.whereType<String>().toSet() ?? {};
    agentCustomModels =
        (j['agentCustomModels'] as List?)
            ?.whereType<String>()
            .toSet()
            .toList() ??
        [];
    agentBunPath = (j['agentBunPath'] as String?) ?? '';
    agentClaudeCodeDir = (j['agentClaudeCodeDir'] as String?) ?? '';
    agentPermissionMode = AgentPermissionMode.values.firstWhere(
      (m) => m.name == j['agentPermissionMode'],
      orElse: () => switch (j['agentApprovals']) {
        'ask' => AgentPermissionMode.manual,
        'allow' => AgentPermissionMode.bypass,
        _ => AgentPermissionMode.auto,
      },
    );
    legacyAgentHost = AgentHost.values
        .where((h) => h.name == j['agentHost'])
        .firstOrNull;
    desktopTools = (j['desktopTools'] as bool?) ?? true;
    desktopRoot = (j['desktopRoot'] as String?) ?? desktopRoot;
    agentMcpServers = {
      for (final e in ((j['agentMcpServers'] as Map?) ?? const {}).entries)
        if (e.key is String && e.value is Map)
          e.key as String: Map<String, dynamic>.from(e.value as Map),
    };
  }

  Map<String, dynamic> toJson() => {
    'agentEnabled': agentEnabled,
    if (legacyAgentModel != null) 'agentModel': legacyAgentModel,
    if (agentModelOverrides.isNotEmpty)
      'agentModelOverrides': {
        for (final e in agentModelOverrides.entries) e.key.runner: e.value,
      },
    if (hubModels != null) 'hubModels': hubModels!.toJson(),
    'agentHiddenModels': agentHiddenModels.toList(),
    'agentCustomModels': agentCustomModels,
    'agentBunPath': agentBunPath,
    'agentClaudeCodeDir': agentClaudeCodeDir,
    'agentPermissionMode': agentPermissionMode.name,
    if (legacyAgentHost != null) 'agentHost': legacyAgentHost!.name,
    'desktopTools': desktopTools,
    'desktopRoot': desktopRoot,
    if (agentMcpServers.isNotEmpty) 'agentMcpServers': agentMcpServers,
  };

  /// Whether the agent answers Ask AI on this device (on, and a platform
  /// that runs it).
  bool get answersAskAi => agentEnabled && agentPlatform;

  /// Applies [change], saves and notifies.
  void update(void Function(AiSettings s) change) {
    change(this);
    _save();
    notifyListeners();
  }

  void _save() {
    final f = file;
    if (f == null) return;
    try {
      f.parent.createSync(recursive: true);
      final tmp = File('${f.path}.tmp');
      tmp.writeAsStringSync(
        const JsonEncoder.withIndent('  ').convert(toJson()),
        flush: true,
      );
      tmp.renameSync(f.path);
    } catch (_) {}
  }
}
