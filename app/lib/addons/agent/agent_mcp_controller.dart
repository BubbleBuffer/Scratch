// What the MCP servers section does, without its layout: the servers' state
// read from the chat engine every [McpServersController.start] interval, the
// editor (a new server or a saved one, its fields and the error that stops a
// save), and the list edits: save, turn on or off, remove. The list itself is
// read and written through [read] and [write]; Scratch keeps it in this
// device's AI settings (`agentMcpServers`), and AgentStore applies it to
// both engines (see McpServersSettings).

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../../src/rust/api/agent.dart' show AgentMcpServer;
import 'agent_mcp_draft.dart';

class McpServersController extends ChangeNotifier {
  McpServersController({
    required this.read,
    required this.write,
    required this.status,
  });

  /// The saved servers.
  final McpServers Function() read;

  /// Saves [next] as the server list.
  final void Function(McpServers next) write;

  /// Each server's state, from the chat engine; null while it can't say.
  final Future<List<AgentMcpServer>?> Function() status;

  Timer? _poll;
  bool _disposed = false;

  /// The servers' state as last read; null until the engine answers.
  List<AgentMcpServer>? get serverStates => _states;
  List<AgentMcpServer>? _states;

  /// Whether the editor is open, for a new server ([original] null) or for
  /// the server named [original].
  bool get editing => _editing;
  bool _editing = false;
  String? get original => _original;
  String? _original;

  /// The editor's server type: a URL, or a command.
  bool get isUrl => _isUrl;
  bool _isUrl = false;
  set isUrl(bool v) {
    _isUrl = v;
    notifyListeners();
  }

  /// Why the last save was refused.
  String? get editorError => _editorError;
  String? _editorError;

  final name = TextEditingController();
  final command = TextEditingController();
  final args = TextEditingController();
  final env = TextEditingController();
  final url = TextEditingController();
  final headers = TextEditingController();

  McpServers get servers => read();

  /// Reads the state now and then every [interval] until disposed.
  void start(Duration interval) {
    unawaited(refresh());
    _poll = Timer.periodic(interval, (_) => refresh());
  }

  /// Reads the servers' state; notifies only when it changed.
  Future<void> refresh() async {
    final next = await status();
    if (_disposed || listEquals(next, _states)) return;
    _states = next;
    notifyListeners();
  }

  /// Opens the editor for the server named [server], or for a new one.
  void edit(String? server) {
    final d = server == null
        ? McpServerDraft()
        : McpServerDraft.of(server, servers[server] ?? const {});
    _editing = true;
    _original = server;
    _isUrl = d.isUrl;
    _editorError = null;
    name.text = d.name;
    command.text = d.command;
    args.text = d.args;
    env.text = d.env;
    url.text = d.url;
    headers.text = d.headers;
    notifyListeners();
  }

  void close() {
    _editing = false;
    _original = null;
    _editorError = null;
    notifyListeners();
  }

  /// The editor's fields as a draft.
  McpServerDraft get draft => McpServerDraft(
    name: name.text.trim(),
    isUrl: _isUrl,
    command: command.text,
    args: args.text,
    env: env.text,
    url: url.text,
    headers: headers.text,
  );

  /// Saves the editor's server, keeping the saved server's other keys, and
  /// closes the editor; or shows why it can't be saved.
  void save() {
    final d = draft;
    final original = _original;
    final saved = servers;
    final error = validateMcpServer(d, saved.keys.where((n) => n != original));
    if (error != null) {
      _editorError = error;
      notifyListeners();
      return;
    }
    final server = mcpServerJson(
      d,
      original == null ? const {} : saved[original] ?? const {},
    );
    write(
      mcpServersWithSaved(
        servers,
        original: original,
        name: d.name,
        server: server,
      ),
    );
    close();
  }

  void setEnabled(String server, bool on) {
    write(mcpServersWithEnabled(servers, server, on));
    notifyListeners();
  }

  void remove(String server) {
    write(mcpServersWithout(servers, server));
    if (_original == server) {
      close();
    } else {
      notifyListeners();
    }
  }

  /// [server]'s state in words, and whether it is a problem; null until the
  /// engine has answered.
  (String, bool)? stateOf(String server, bool enabled) {
    final list = _states;
    if (list == null) return null;
    final s = list.where((x) => x.name == server).firstOrNull;
    if (s == null) return enabled ? ('Not started', false) : ('Off', false);
    return switch (s.state) {
      'running' => (
        'Running, ${s.tools} ${s.tools == 1 ? 'tool' : 'tools'}',
        false,
      ),
      'failed' => (
        s.error == null || s.error!.trim().isEmpty
            ? 'Failed to start'
            : 'Failed: ${s.error!.trim()}',
        true,
      ),
      _ => ('Off', false),
    };
  }

  @override
  void dispose() {
    _disposed = true;
    _poll?.cancel();
    for (final c in [name, command, args, env, url, headers]) {
      c.dispose();
    }
    super.dispose();
  }
}
