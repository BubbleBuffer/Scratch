// AI settings of this device: which runner does AI tasks here and where
// its program is, the DeepSeek key, the vault's MCP endpoint and the
// in-app agent (on or off, where it runs, default model, Claude Code
// extension, approvals). Kept in
// `<data dir>/ai_settings.json` next to settings.json (per device, never
// synced; the DeepSeek key stays on this device).

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

/// Who does AI tasks on this device.
enum AiRunner {
  none('Off'),
  agent('Agent'),
  claude('Claude Code'),
  codex('Codex'),
  deepseek('DeepSeek API');

  const AiRunner(this.label);
  final String label;

  /// Command-line agents get the vault's MCP tools; DeepSeek only the
  /// page's text. The in-app agent calls the vault tools directly.
  bool get usesMcp => this == claude || this == codex;
}

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
  local('This computer'),
  hub('Hub');

  const AgentHost(this.label);
  final String label;
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

/// The MCP endpoint's default address: this computer only.
const defaultMcpAddress = '127.0.0.1:47101';

const defaultDeepSeekUrl = 'https://api.deepseek.com';
const defaultDeepSeekModel = 'deepseek-chat';

class AiSettings extends ChangeNotifier {
  AiSettings({this.file});

  /// Where the settings are saved (null: in memory only).
  final File? file;

  /// The runner the user chose; null until one is chosen, when the agent
  /// (if on) or nothing runs tasks.
  AiRunner? _runner;
  AiRunner get runner =>
      _runner ?? (agentEnabled ? AiRunner.agent : AiRunner.none);
  set runner(AiRunner r) => _runner = r;

  /// Full paths of the agents' programs (empty: found on PATH).
  String claudePath = '';
  String codexPath = '';

  String deepSeekKey = '';
  String deepSeekModel = defaultDeepSeekModel;
  String deepSeekUrl = defaultDeepSeekUrl;

  /// Serve the vault's MCP endpoint on [mcpAddress] so AI apps (Claude,
  /// Codex, ChatGPT through a tunnel) can use the vault while the app runs.
  bool mcpEnabled = false;
  String mcpAddress = defaultMcpAddress;

  /// Minutes an agent may run before it is stopped.
  int timeoutMinutes = 10;

  /// The in-app agent (local on desktop, always available through the Hub
  /// on Android, including installs whose old settings saved it as off).
  bool agentEnabled = agentPlatform;

  /// `provider/model`; empty: the agent's built-in default.
  String agentModel = '';

  /// Model visibility and extra provider/model ids, on this device for either host.
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

  /// Where new chats run ("Agent runs on"): here, or on the hub this vault
  /// syncs with (only when that is an `https://` hub with a device token).
  /// A chat can switch before its first message and then keeps its host.
  // Android's effective host is always Hub; this preference is desktop-only.
  AgentHost agentHost = AgentHost.local;

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
    _runner = AiRunner.values.where((r) => r.name == j['runner']).firstOrNull;
    claudePath = (j['claudePath'] as String?) ?? '';
    codexPath = (j['codexPath'] as String?) ?? '';
    deepSeekKey = (j['deepSeekKey'] as String?) ?? '';
    deepSeekModel = (j['deepSeekModel'] as String?) ?? defaultDeepSeekModel;
    deepSeekUrl = (j['deepSeekUrl'] as String?) ?? defaultDeepSeekUrl;
    mcpEnabled = (j['mcpEnabled'] as bool?) ?? false;
    mcpAddress = (j['mcpAddress'] as String?) ?? defaultMcpAddress;
    timeoutMinutes = (j['timeoutMinutes'] as int?) ?? 10;
    agentEnabled =
        hubAgentPlatform ||
        (localAgentPlatform && ((j['agentEnabled'] as bool?) ?? true));
    agentModel = (j['agentModel'] as String?) ?? '';
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
    agentHost = AgentHost.values.firstWhere(
      (h) => h.name == j['agentHost'],
      orElse: () => AgentHost.local,
    );
    desktopTools = (j['desktopTools'] as bool?) ?? true;
    desktopRoot = (j['desktopRoot'] as String?) ?? desktopRoot;
  }

  Map<String, dynamic> toJson() => {
    if (_runner != null) 'runner': _runner!.name,
    'claudePath': claudePath,
    'codexPath': codexPath,
    'deepSeekKey': deepSeekKey,
    'deepSeekModel': deepSeekModel,
    'deepSeekUrl': deepSeekUrl,
    'mcpEnabled': mcpEnabled,
    'mcpAddress': mcpAddress,
    'timeoutMinutes': timeoutMinutes,
    'agentEnabled': agentEnabled,
    'agentModel': agentModel,
    'agentHiddenModels': agentHiddenModels.toList(),
    'agentCustomModels': agentCustomModels,
    'agentBunPath': agentBunPath,
    'agentClaudeCodeDir': agentClaudeCodeDir,
    'agentPermissionMode': agentPermissionMode.name,
    'agentHost': agentHost.name,
    'desktopTools': desktopTools,
    'desktopRoot': desktopRoot,
  };

  /// Whether [r] can run here as configured (DeepSeek needs a key).
  bool ready(AiRunner r) => switch (r) {
    AiRunner.none => false,
    AiRunner.agent => agentEnabled && agentPlatform,
    AiRunner.deepseek => deepSeekKey.trim().isNotEmpty,
    _ => true,
  };

  /// The program override for a command-line runner (null: on PATH).
  String? programOf(AiRunner r) {
    final p = switch (r) {
      AiRunner.claude => claudePath,
      AiRunner.codex => codexPath,
      _ => '',
    };
    return p.trim().isEmpty ? null : p.trim();
  }

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
