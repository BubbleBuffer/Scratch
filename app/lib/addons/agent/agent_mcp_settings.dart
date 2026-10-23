// Settings > Connectors > Tools for the agent (desktop): the MCP servers the
// agent on this computer starts, for in-app chats and the host relay. Each
// server is a command (with arguments and environment) or a URL (with
// headers whose values come from environment variables), and can be turned
// off. They start only while this computer's tool set is Everything; with
// Read or Write the row shows a lock line linking to Settings > Agent. The
// list is this device's (AI settings `agentMcpServers`); changes apply to
// both engines at once ([AgentStore]), and each server's state is read from
// the chat engine every [McpServersSettings.statusInterval] while shown.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

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

/// Marks the MCP servers on Settings > Connectors, so other places can
/// scroll to them ([showMcpServers]).
final mcpServersAnchor = GlobalKey(debugLabel: 'mcp-servers');

/// The tools panel's status: "3 servers · 1 off", or "No servers".
String mcpServersSummary(AgentStore store) {
  final all = store.settings.agentMcpServers.values;
  if (all.isEmpty) return 'No servers';
  final off = all.where((v) => v['enabled'] == false).length;
  final n = '${all.length} ${all.length == 1 ? 'server' : 'servers'}';
  return off == 0 ? n : '$n · $off off';
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
    final store = widget.store;
    final everything =
        store.vaultDefaults.toolSetFor(AgentHost.local) == 'everything';
    final errors = [
      if (store.mcpError != null)
        "Couldn't apply the servers to chats: ${store.mcpError}",
      if (store.relay.mcpError != null)
        "Couldn't apply the servers to the hub relay: ${store.relay.mcpError}",
    ];
    final note = _status == null && _servers.isNotEmpty
        ? 'Their state shows while the agent runs on this computer.'
        : null;
    Widget add() => SButton(
      'Add server',
      key: const ValueKey('agent-mcp-add'),
      icon: Sx.plus,
      onTap: () => _edit(null),
    );
    final body = !everything
        ? null
        : Column(
            key: const ValueKey('agent-mcp-servers'),
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (_servers.isNotEmpty)
                SList(
                  foot: _editing ? null : SListFoot(children: [add()]),
                  children: [
                    for (final e in _servers.entries)
                      _row(context, e.key, e.value),
                  ],
                )
              else if (!_editing)
                SEmpty(
                  icon: Sx.wrench,
                  title: 'No MCP servers',
                  text:
                      'MCP servers are small programs or web services that give the agent extra tools, such as reading a database or searching documentation. Their setup instructions give you a command or a URL to add here.',
                  action: add(),
                ),
              if (note != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    note,
                    key: const ValueKey('agent-mcp-note'),
                    style: sHint(context),
                  ),
                ),
              if (errors.isNotEmpty)
                SNotice(
                  errors.join('\n'),
                  kind: SNoticeKind.bad,
                  textKey: const ValueKey('agent-mcp-error'),
                ),
              if (_editing) _editor(context),
            ],
          );
    return SRow(
      label: 'MCP servers',
      desc:
          'Extra tools for the agent, from programs on this computer or from web addresses. Changes apply at once.',
      help:
          'Add one when a tool or service offers an MCP server and you want the agent to use it.',
      stack: true,
      disabled: !everything,
      reason: everything
          ? null
          : 'Needs “Everything” on the Agent page. Servers don’t start with Read or Write.',
      reasonActionLabel: everything ? null : 'Change on the Agent page',
      reasonAction: everything
          ? null
          : () => openSettingsSection(context, SettingsSection.agent),
      below: body,
    );
  }

  Future<void> _confirmRemove(String name) async {
    final ok = await showConfirm(
      context,
      title: 'Remove “$name”?',
      message:
          'The agent loses the tools from this MCP server at once. You can add it again later.',
      confirmLabel: 'Remove server',
      danger: true,
    );
    if (ok && mounted) _remove(name);
  }

  Widget _row(BuildContext context, String name, Map<String, dynamic> json) {
    final enabled = json['enabled'] != false;
    final d = McpServerDraft.of(name, json);
    final what = d.isUrl
        ? d.url
        : [d.command, ..._lines(d.args)].where((p) => p.isNotEmpty).join(' ');
    final state = _stateOf(name, enabled);
    final failed = state?.$2 ?? false;
    final pill = state == null
        ? null
        : SStatus(
            failed ? 'Failed' : state.$1,
            key: ValueKey('agent-mcp-status-$name'),
            kind: failed
                ? SStatusKind.bad
                : state.$1.startsWith('Running')
                ? SStatusKind.ok
                : SStatusKind.off,
          );
    return KeyedSubtree(
      key: ValueKey('agent-mcp-server-$name'),
      child: SItem(
        icon: d.isUrl ? Sx.globe : Sx.terminal,
        name: name,
        tags: [d.isUrl ? 'URL' : 'Program'],
        status: pill,
        sub: what,
        subMono: true,
        error: failed ? state!.$1 : null,
        trailing: [
          SSwitch(
            key: ValueKey('agent-mcp-enabled-$name'),
            label: '$name on',
            value: enabled,
            onChanged: (on) => _setEnabled(name, on),
          ),
          const SizedBox(width: 4),
          SIconButton(
            Sx.pencil,
            key: ValueKey('agent-mcp-edit-$name'),
            label: 'Edit $name',
            onTap: () => _edit(name),
          ),
          SIconButton(
            Sx.trash,
            key: ValueKey('agent-mcp-remove-$name'),
            label: 'Remove $name',
            danger: true,
            onTap: () => _confirmRemove(name),
          ),
        ],
      ),
    );
  }

  Widget _editor(BuildContext context) {
    return SEditor(
      key: const ValueKey('agent-mcp-editor'),
      title: _original == null ? 'Add an MCP server' : 'Edit $_original',
      actions: [
        SButton(
          'Cancel',
          key: const ValueKey('agent-mcp-cancel'),
          kind: SButtonKind.quiet,
          onTap: _close,
        ),
        SButton(
          _original == null ? 'Add server' : 'Save',
          key: const ValueKey('agent-mcp-save'),
          kind: SButtonKind.primary,
          onTap: _save,
        ),
      ],
      children: [
        SField(
          label: 'How it runs',
          child: Align(
            alignment: Alignment.centerLeft,
            child: SSegmented<bool>(
              key: const ValueKey('agent-mcp-type'),
              value: _isUrl,
              options: const [false, true],
              keyOf: (url) => ValueKey(
                url ? 'agent-mcp-type-url' : 'agent-mcp-type-command',
              ),
              labelOf: (url) =>
                  url ? 'A web address' : 'A program on this computer',
              onChanged: (url) => setState(() => _isUrl = url),
            ),
          ),
        ),
        SField(
          label: 'Name',
          child: STextField(
            fieldKey: const ValueKey('agent-mcp-name'),
            controller: _name,
            placeholder: 'filesystem',
            autofocus: true,
          ),
        ),
        if (_isUrl) ...[
          SField(
            label: 'URL',
            child: STextField(
              fieldKey: const ValueKey('agent-mcp-url'),
              controller: _url,
              placeholder: 'https://example.com/mcp',
              mono: true,
            ),
          ),
          SField(
            label: 'Headers, Header=ENV_VAR per line (optional)',
            hint:
                'Each value is read from that environment variable on this computer. Its name ends in _API_KEY, _KEY or _TOKEN.',
            child: _LinesField(
              key: const ValueKey('agent-mcp-headers'),
              controller: _headers,
              placeholder: 'Authorization=EXAMPLE_TOKEN',
            ),
          ),
        ] else ...[
          SField(
            label: 'Command',
            child: STextField(
              fieldKey: const ValueKey('agent-mcp-command'),
              controller: _command,
              placeholder: 'npx',
              mono: true,
            ),
          ),
          SField(
            label: 'Arguments, one per line',
            child: _LinesField(
              key: const ValueKey('agent-mcp-args'),
              controller: _args,
              placeholder: '-y\n@example/mcp-server',
            ),
          ),
          SField(
            label: 'Environment variables, KEY=value per line (optional)',
            hint:
                'Copy these from the server’s setup instructions. They usually give a command such as npx -y @org/server.',
            child: _LinesField(
              key: const ValueKey('agent-mcp-env'),
              controller: _env,
              placeholder: 'EXAMPLE_ROOT=/path',
            ),
          ),
        ],
        if (_editorError != null)
          SNotice(
            _editorError!,
            kind: SNoticeKind.bad,
            textKey: const ValueKey('agent-mcp-editor-error'),
          ),
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
