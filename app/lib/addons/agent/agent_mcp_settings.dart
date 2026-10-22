// Settings > AI > Agent (desktop), next to the tool sets: the MCP servers the
// agent on this computer starts, for in-app chats and the host relay. Each
// server is a command (with arguments and environment) or a URL (with
// headers whose values come from environment variables), and can be turned
// off. The list is this device's (AI settings `agentMcpServers`); changes
// apply to both engines at once ([AgentStore]), and each server's state is
// read from the chat engine every [McpServersSettings.statusInterval] while
// the section is shown.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';

/// One server as the editor shows it.
class McpServerDraft {
  McpServerDraft({
    this.name = '',
    this.isUrl = false,
    this.command = '',
    this.args = '',
    this.env = '',
    this.url = '',
    this.headers = '',
  });

  /// [json] (BubblePaw's `McpServerSettings`) named [name], as text.
  factory McpServerDraft.of(String name, Map<String, dynamic> json) {
    String lines(Object? map) => map is Map
        ? [for (final e in map.entries) '${e.key}=${e.value}'].join('\n')
        : '';
    final args = json['args'];
    return McpServerDraft(
      name: name,
      isUrl: json['url'] is String,
      command: json['command'] is String ? json['command'] as String : '',
      args: args is List ? args.map((a) => '$a').join('\n') : '',
      env: lines(json['env']),
      url: json['url'] is String ? json['url'] as String : '',
      headers: lines(json['headers_from']),
    );
  }

  String name, command, args, env, url, headers;
  bool isUrl;
}

/// Non-empty trimmed lines of [text].
List<String> _lines(String text) => [
  for (final l in text.split('\n'))
    if (l.trim().isNotEmpty) l.trim(),
];

/// `KEY=value` lines as a map; null when a line has no `=` or no key.
Map<String, String>? _pairs(String text) {
  final out = <String, String>{};
  for (final l in _lines(text)) {
    final i = l.indexOf('=');
    if (i <= 0) return null;
    out[l.substring(0, i).trim()] = l.substring(i + 1).trim();
  }
  return out;
}

/// Why [d] can't be saved among [names] (the other servers' names), or null.
String? validateMcpServer(McpServerDraft d, Iterable<String> names) {
  final name = d.name.trim();
  if (name.isEmpty) return 'Give the server a name.';
  if (names.contains(name)) return 'A server named $name already exists.';
  if (d.isUrl) {
    if (d.url.trim().isEmpty) return 'Enter the server URL.';
    final headers = _pairs(d.headers);
    if (headers == null) {
      return 'Write each header as Header=ENV_VAR, one per line.';
    }
    for (final e in headers.entries) {
      final v = e.value;
      if (v.isEmpty) return 'Header ${e.key} needs an environment variable.';
      if (!v.endsWith('_API_KEY') &&
          !v.endsWith('_KEY') &&
          !v.endsWith('_TOKEN')) {
        return '$v must end in _API_KEY, _KEY or _TOKEN.';
      }
    }
  } else {
    if (d.command.trim().isEmpty) return 'Enter the command that starts it.';
    if (_pairs(d.env) == null) {
      return 'Write each environment variable as KEY=value, one per line.';
    }
  }
  return null;
}

/// [d] as BubblePaw JSON, on top of [base] (the server as saved), so keys
/// the editor doesn't show stay as they are. Keys of the other kind go:
/// a URL server can't have a command, arguments, environment or folder.
Map<String, dynamic> mcpServerJson(
  McpServerDraft d, [
  Map<String, dynamic> base = const {},
]) {
  final out = Map<String, dynamic>.of(base);
  out.putIfAbsent('enabled', () => true);
  if (d.isUrl) {
    for (final k in ['command', 'args', 'env', 'env_from', 'cwd']) {
      out.remove(k);
    }
    out['url'] = d.url.trim();
    final headers = _pairs(d.headers) ?? const {};
    if (headers.isEmpty) {
      out.remove('headers_from');
    } else {
      out['headers_from'] = headers;
    }
  } else {
    for (final k in ['url', 'headers_from']) {
      out.remove(k);
    }
    out['command'] = d.command.trim();
    final args = _lines(d.args);
    if (args.isEmpty) {
      out.remove('args');
    } else {
      out['args'] = args;
    }
    final env = _pairs(d.env) ?? const {};
    if (env.isEmpty) {
      out.remove('env');
    } else {
      out['env'] = env;
    }
  }
  return out;
}

class McpServersSettings extends StatefulWidget {
  const McpServersSettings({super.key, required this.store});
  final AgentStore store;

  /// How often the servers' state is read while the section is shown.
  static const statusInterval = Duration(seconds: 2);

  @override
  State<McpServersSettings> createState() => _McpServersSettingsState();
}

class _McpServersSettingsState extends State<McpServersSettings> {
  Timer? _poll;
  List<AgentMcpServer>? _status;

  /// The editor: open for a new server ([_original] null) or for the server
  /// named [_original].
  bool _editing = false;
  String? _original;
  bool _isUrl = false;
  String? _editorError;
  final _name = TextEditingController();
  final _command = TextEditingController();
  final _args = TextEditingController();
  final _env = TextEditingController();
  final _url = TextEditingController();
  final _headers = TextEditingController();

  @override
  void initState() {
    super.initState();
    unawaited(_read());
    _poll = Timer.periodic(McpServersSettings.statusInterval, (_) => _read());
  }

  @override
  void dispose() {
    _poll?.cancel();
    for (final c in [_name, _command, _args, _env, _url, _headers]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _read() async {
    final next = await widget.store.mcpStatus();
    if (!mounted || listEquals(next, _status)) return;
    setState(() => _status = next);
  }

  Map<String, Map<String, dynamic>> get _servers =>
      widget.store.settings.agentMcpServers;

  void _edit(String? name) {
    final d = name == null
        ? McpServerDraft()
        : McpServerDraft.of(name, _servers[name] ?? const {});
    setState(() {
      _editing = true;
      _original = name;
      _isUrl = d.isUrl;
      _editorError = null;
      _name.text = d.name;
      _command.text = d.command;
      _args.text = d.args;
      _env.text = d.env;
      _url.text = d.url;
      _headers.text = d.headers;
    });
  }

  void _close() => setState(() {
    _editing = false;
    _original = null;
    _editorError = null;
  });

  void _save() {
    final d = McpServerDraft(
      name: _name.text.trim(),
      isUrl: _isUrl,
      command: _command.text,
      args: _args.text,
      env: _env.text,
      url: _url.text,
      headers: _headers.text,
    );
    final original = _original;
    final error = validateMcpServer(
      d,
      _servers.keys.where((n) => n != original),
    );
    if (error != null) {
      setState(() => _editorError = error);
      return;
    }
    final server = mcpServerJson(
      d,
      original == null ? const {} : _servers[original] ?? const {},
    );
    // Same place in the list; a rename keeps it there.
    widget.store.settings.update((s) {
      final next = <String, Map<String, dynamic>>{};
      for (final e in s.agentMcpServers.entries) {
        next[e.key == original ? d.name : e.key] = e.key == original
            ? server
            : e.value;
      }
      next.putIfAbsent(d.name, () => server);
      s.agentMcpServers = next;
    });
    _close();
  }

  void _setEnabled(String name, bool on) {
    widget.store.settings.update((s) {
      s.agentMcpServers = {
        for (final e in s.agentMcpServers.entries)
          e.key: e.key == name
              ? (Map<String, dynamic>.of(e.value)..['enabled'] = on)
              : e.value,
      };
    });
    setState(() {});
  }

  void _remove(String name) {
    widget.store.settings.update(
      (s) => s.agentMcpServers = {
        for (final e in s.agentMcpServers.entries)
          if (e.key != name) e.key: e.value,
      },
    );
    if (_original == name) {
      _close();
    } else {
      setState(() {});
    }
  }

  /// The server's state in words, and whether it is a problem.
  (String, bool)? _stateOf(String name, bool enabled) {
    final list = _status;
    if (list == null) return null;
    final s = list.where((x) => x.name == name).firstOrNull;
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
  Widget build(BuildContext context) {
    final c = context.colors;
    final store = widget.store;
    final everything =
        store.vaultDefaults.toolSetFor(AgentHost.local) == 'everything';
    final errors = [
      if (store.mcpError != null)
        "Couldn't apply the servers to chats: ${store.mcpError}",
      if (store.relay.mcpError != null)
        "Couldn't apply the servers to the hub relay: ${store.relay.mcpError}",
    ];
    final note = !everything
        ? 'Servers run only while Tools on this computer is '
              '${toolSetLabel('everything')}.'
        : _status == null
        ? 'Their state shows while the agent runs on this computer.'
        : null;
    return SettingsRow(
      label: 'MCP servers',
      description:
          'Programs and URLs that give the agent on this computer more tools, '
          'in chats and for the hub. Saved on this device; changes apply at once.',
      stack: true,
      trailing: Column(
        key: const ValueKey('agent-mcp-servers'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final e in _servers.entries)
            _row(context, e.key, e.value, showState: everything),
          if (note != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                note,
                key: const ValueKey('agent-mcp-note'),
                style: tsRaw(12, 16, 400, c.textSecondary),
              ),
            ),
          if (errors.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                errors.join('\n'),
                key: const ValueKey('agent-mcp-error'),
                style: tsRaw(12, 16, 400, c.inlineCodeText),
              ),
            ),
          if (_editing)
            _editor(context)
          else
            Align(
              alignment: Alignment.centerLeft,
              child: TextBtn(
                'Add server',
                key: const ValueKey('agent-mcp-add'),
                onTap: () => _edit(null),
              ),
            ),
        ],
      ),
    );
  }

  Widget _row(
    BuildContext context,
    String name,
    Map<String, dynamic> json, {
    required bool showState,
  }) {
    final c = context.colors;
    final enabled = json['enabled'] != false;
    final d = McpServerDraft.of(name, json);
    final what = d.isUrl
        ? d.url
        : [d.command, ..._lines(d.args)].where((p) => p.isNotEmpty).join(' ');
    final state = showState ? _stateOf(name, enabled) : null;
    return Padding(
      key: ValueKey('agent-mcp-server-$name'),
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(name, style: tsRaw(14, 20, 400, c.textPrimary)),
                if (what.isNotEmpty)
                  Text(
                    what,
                    style: ts(
                      const TypeSpec(12, 16, 400),
                      c.textSecondary,
                      mono: true,
                    ),
                    softWrap: true,
                  ),
                if (state != null)
                  Text(
                    state.$1,
                    key: ValueKey('agent-mcp-status-$name'),
                    style: tsRaw(
                      12,
                      16,
                      400,
                      state.$2 ? c.inlineCodeText : c.textSecondary,
                    ),
                    softWrap: true,
                  ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          TextBtn(
            'Edit',
            key: ValueKey('agent-mcp-edit-$name'),
            onTap: () => _edit(name),
          ),
          TextBtn(
            'Remove',
            key: ValueKey('agent-mcp-remove-$name'),
            color: c.inlineCodeText,
            onTap: () => _remove(name),
          ),
          const SizedBox(width: 8),
          Toggle(
            key: ValueKey('agent-mcp-enabled-$name'),
            value: enabled,
            onChanged: (on) => _setEnabled(name, on),
          ),
        ],
      ),
    );
  }

  Widget _editor(BuildContext context) {
    final c = context.colors;
    Widget label(String text) => Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 4),
      child: Text(text, style: tsRaw(12, 16, 400, c.textSecondary)),
    );
    return Column(
      key: const ValueKey('agent-mcp-editor'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: SettingsField(
                fieldKey: const ValueKey('agent-mcp-name'),
                controller: _name,
                placeholder: 'Name',
                autofocus: true,
              ),
            ),
            const SizedBox(width: 8),
            SelectButton<bool>(
              key: const ValueKey('agent-mcp-type'),
              title: 'Server type',
              value: _isUrl,
              options: const [false, true],
              labelOf: (url) => url ? 'URL' : 'Command',
              onChanged: (url) => setState(() => _isUrl = url),
            ),
          ],
        ),
        if (_isUrl) ...[
          label('URL'),
          SettingsField(
            fieldKey: const ValueKey('agent-mcp-url'),
            controller: _url,
            placeholder: 'https://example.com/mcp',
            mono: true,
          ),
          label(
            'Headers, one per line as Header=ENV_VAR. Each value is read from '
            'that environment variable on this computer; its name ends in '
            '_API_KEY, _KEY or _TOKEN.',
          ),
          _LinesField(
            key: const ValueKey('agent-mcp-headers'),
            controller: _headers,
            placeholder: 'Authorization=EXAMPLE_TOKEN',
          ),
        ] else ...[
          label('Command'),
          SettingsField(
            fieldKey: const ValueKey('agent-mcp-command'),
            controller: _command,
            placeholder: 'npx',
            mono: true,
          ),
          label('Arguments, one per line'),
          _LinesField(
            key: const ValueKey('agent-mcp-args'),
            controller: _args,
            placeholder: '-y\n@example/mcp-server',
          ),
          label('Environment, one per line as KEY=value'),
          _LinesField(
            key: const ValueKey('agent-mcp-env'),
            controller: _env,
            placeholder: 'EXAMPLE_ROOT=/path',
          ),
        ],
        if (_editorError != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              _editorError!,
              key: const ValueKey('agent-mcp-editor-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ),
        const SizedBox(height: 8),
        Row(
          children: [
            SecondaryButton(
              label: _original == null ? 'Add' : 'Save',
              key: const ValueKey('agent-mcp-save'),
              height: 32,
              onTap: _save,
            ),
            const SizedBox(width: 8),
            TextBtn(
              'Cancel',
              key: const ValueKey('agent-mcp-cancel'),
              onTap: _close,
            ),
          ],
        ),
        const SizedBox(height: 8),
      ],
    );
  }
}

/// A bordered multi-line monospace field, like [SettingsField].
class _LinesField extends StatefulWidget {
  const _LinesField({
    super.key,
    required this.controller,
    required this.placeholder,
  });
  final TextEditingController controller;
  final String placeholder;

  @override
  State<_LinesField> createState() => _LinesFieldState();
}

class _LinesFieldState extends State<_LinesField> {
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_changed);
  }

  @override
  void didUpdateWidget(_LinesField old) {
    super.didUpdateWidget(old);
    if (!identical(old.controller, widget.controller)) {
      old.controller.removeListener(_changed);
      widget.controller.addListener(_changed);
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    _focus.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final style = ts(
      TypeSpec(context.mobile ? 14 : 13, 20, 400),
      c.textPrimary,
      mono: true,
    );
    return GestureDetector(
      onTap: _focus.requestFocus,
      child: FieldFrame(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: Stack(
          children: [
            if (widget.controller.text.isEmpty)
              Positioned.fill(
                child: IgnorePointer(
                  child: Text(
                    widget.placeholder,
                    style: style.copyWith(color: c.textTertiary),
                  ),
                ),
              ),
            EditableText(
              controller: widget.controller,
              focusNode: _focus,
              style: style,
              cursorColor: c.textPrimary,
              backgroundCursorColor: c.textTertiary,
              selectionColor: c.textSelection,
              cursorWidth: 1.5,
              minLines: 2,
              maxLines: 8,
              keyboardType: TextInputType.multiline,
              selectionControls: noteSelectionControls,
            ),
          ],
        ),
      ),
    );
  }
}
