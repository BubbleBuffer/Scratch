// Settings > Connectors > Tools for the agent (desktop): the MCP servers the
// agent on this computer starts, for in-app chats and the host relay. Each
// server is a command (with arguments and environment) or a URL (with
// headers whose values come from environment variables), and can be turned
// off. They start only while this computer's tool set is Everything; with
// Read or Write the row shows a lock line linking to Settings > Agent. The
// list is this device's (AI settings `agentMcpServers`); changes apply to
// both engines at once ([AgentStore]), and each server's state is read from
// the chat engine every [McpServersSettings.statusInterval] while shown.
// What the section does is [McpServersController]; the editor's data and the
// JSON format are in agent_mcp_draft.dart. This file is the layout.

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_mcp_controller.dart';
import 'agent_mcp_draft.dart';
import 'agent_store.dart';

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
  // Reads [widget]'s store each time, so a new store is picked up.
  late final _mcp = McpServersController(
    read: () => widget.store.settings.agentMcpServers,
    write: (next) =>
        widget.store.settings.update((s) => s.agentMcpServers = next),
    status: () => widget.store.mcpStatus(),
  );

  @override
  void initState() {
    super.initState();
    _mcp.addListener(_changed);
    _mcp.start(McpServersSettings.statusInterval);
  }

  @override
  void dispose() {
    _mcp.removeListener(_changed);
    _mcp.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  Future<void> _confirmRemove(String name) async {
    final ok = await showConfirm(
      context,
      title: 'Remove “$name”?',
      message:
          'The agent loses the tools from this MCP server at once. You can add it again later.',
      confirmLabel: 'Remove server',
      danger: true,
    );
    if (ok && mounted) _mcp.remove(name);
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final servers = _mcp.servers;
    final everything =
        store.vaultDefaults.toolSetFor(AgentHost.local) == 'everything';
    final errors = [
      if (store.mcpError != null)
        "Couldn't apply the servers to chats: ${store.mcpError}",
      if (store.relay.mcpError != null)
        "Couldn't apply the servers to the hub relay: ${store.relay.mcpError}",
    ];
    final note = _mcp.serverStates == null && servers.isNotEmpty
        ? 'Their state shows while the agent runs on this computer.'
        : null;
    Widget add() => SButton(
      'Add server',
      key: const ValueKey('agent-mcp-add'),
      icon: Sx.plus,
      onTap: () => _mcp.edit(null),
    );
    final body = !everything
        ? null
        : Column(
            key: const ValueKey('agent-mcp-servers'),
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (servers.isNotEmpty)
                SList(
                  foot: _mcp.editing ? null : SListFoot(children: [add()]),
                  children: [
                    for (final e in servers.entries)
                      _row(context, e.key, e.value),
                  ],
                )
              else if (!_mcp.editing)
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
              if (_mcp.editing) _editor(context),
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

  Widget _row(BuildContext context, String name, Map<String, dynamic> json) {
    final enabled = json['enabled'] != false;
    final d = McpServerDraft.of(name, json);
    final state = _mcp.stateOf(name, enabled);
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
        sub: d.summary,
        subMono: true,
        error: failed ? state!.$1 : null,
        trailing: [
          SSwitch(
            key: ValueKey('agent-mcp-enabled-$name'),
            label: '$name on',
            value: enabled,
            onChanged: (on) => _mcp.setEnabled(name, on),
          ),
          const SizedBox(width: 4),
          SIconButton(
            Sx.pencil,
            key: ValueKey('agent-mcp-edit-$name'),
            label: 'Edit $name',
            onTap: () => _mcp.edit(name),
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
      title: _mcp.original == null ? 'Add an MCP server' : 'Edit $_mcp.original',
      actions: [
        SButton(
          'Cancel',
          key: const ValueKey('agent-mcp-cancel'),
          kind: SButtonKind.quiet,
          onTap: _mcp.close,
        ),
        SButton(
          _mcp.original == null ? 'Add server' : 'Save',
          key: const ValueKey('agent-mcp-save'),
          kind: SButtonKind.primary,
          onTap: _mcp.save,
        ),
      ],
      children: [
        SField(
          label: 'How it runs',
          child: Align(
            alignment: Alignment.centerLeft,
            child: SSegmented<bool>(
              key: const ValueKey('agent-mcp-type'),
              value: _mcp.isUrl,
              options: const [false, true],
              keyOf: (url) => ValueKey(
                url ? 'agent-mcp-type-url' : 'agent-mcp-type-command',
              ),
              labelOf: (url) =>
                  url ? 'A web address' : 'A program on this computer',
              onChanged: (url) => _mcp.isUrl = url,
            ),
          ),
        ),
        SField(
          label: 'Name',
          child: STextField(
            fieldKey: const ValueKey('agent-mcp-name'),
            controller: _mcp.name,
            placeholder: 'filesystem',
            autofocus: true,
          ),
        ),
        if (_mcp.isUrl) ...[
          SField(
            label: 'URL',
            child: STextField(
              fieldKey: const ValueKey('agent-mcp-url'),
              controller: _mcp.url,
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
              controller: _mcp.headers,
              placeholder: 'Authorization=EXAMPLE_TOKEN',
            ),
          ),
        ] else ...[
          SField(
            label: 'Command',
            child: STextField(
              fieldKey: const ValueKey('agent-mcp-command'),
              controller: _mcp.command,
              placeholder: 'npx',
              mono: true,
            ),
          ),
          SField(
            label: 'Arguments, one per line',
            child: _LinesField(
              key: const ValueKey('agent-mcp-args'),
              controller: _mcp.args,
              placeholder: '-y\n@example/mcp-server',
            ),
          ),
          SField(
            label: 'Environment variables, KEY=value per line (optional)',
            hint:
                'Copy these from the server’s setup instructions. They usually give a command such as npx -y @org/server.',
            child: _LinesField(
              key: const ValueKey('agent-mcp-env'),
              controller: _mcp.env,
              placeholder: 'EXAMPLE_ROOT=/path',
            ),
          ),
        ],
        if (_mcp.editorError != null)
          SNotice(
            _mcp.editorError!,
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
