// Settings > AI > Agent (desktop), next to the tool sets: the MCP servers the
// agent on this computer starts, for in-app chats and the host relay. Each
// server is a command (with arguments and environment) or a URL (with
// headers whose values come from environment variables), and can be turned
// off. The list is this device's (AI settings `agentMcpServers`); changes
// apply to both engines at once ([AgentStore]), and each server's state is
// read from the chat engine every [McpServersSettings.statusInterval] while
// the section is shown. What the section does is [McpServersController]; the
// editor's data and the JSON format are in agent_mcp_draft.dart. This file
// is the layout.

import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_mcp_controller.dart';
import 'agent_mcp_draft.dart';
import 'agent_store.dart';

/// Marks the MCP servers in Settings > AI, so Settings > Sync can scroll to
/// them ([showMcpServers]).
final mcpServersAnchor = GlobalKey(debugLabel: 'mcp-servers');

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
        : _mcp.serverStates == null
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
          for (final e in _mcp.servers.entries)
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
          if (_mcp.editing)
            _editor(context)
          else
            Align(
              alignment: Alignment.centerLeft,
              child: TextBtn(
                'Add server',
                key: const ValueKey('agent-mcp-add'),
                onTap: () => _mcp.edit(null),
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
    final what = McpServerDraft.of(name, json).summary;
    final state = showState ? _mcp.stateOf(name, enabled) : null;
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
            onTap: () => _mcp.edit(name),
          ),
          TextBtn(
            'Remove',
            key: ValueKey('agent-mcp-remove-$name'),
            color: c.inlineCodeText,
            onTap: () => _mcp.remove(name),
          ),
          const SizedBox(width: 8),
          Toggle(
            key: ValueKey('agent-mcp-enabled-$name'),
            value: enabled,
            onChanged: (on) => _mcp.setEnabled(name, on),
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
                controller: _mcp.name,
                placeholder: 'Name',
                autofocus: true,
              ),
            ),
            const SizedBox(width: 8),
            SelectButton<bool>(
              key: const ValueKey('agent-mcp-type'),
              title: 'Server type',
              value: _mcp.isUrl,
              options: const [false, true],
              labelOf: (url) => url ? 'URL' : 'Command',
              onChanged: (url) => _mcp.isUrl = url,
            ),
          ],
        ),
        if (_mcp.isUrl) ...[
          label('URL'),
          SettingsField(
            fieldKey: const ValueKey('agent-mcp-url'),
            controller: _mcp.url,
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
            controller: _mcp.headers,
            placeholder: 'Authorization=EXAMPLE_TOKEN',
          ),
        ] else ...[
          label('Command'),
          SettingsField(
            fieldKey: const ValueKey('agent-mcp-command'),
            controller: _mcp.command,
            placeholder: 'npx',
            mono: true,
          ),
          label('Arguments, one per line'),
          _LinesField(
            key: const ValueKey('agent-mcp-args'),
            controller: _mcp.args,
            placeholder: '-y\n@example/mcp-server',
          ),
          label('Environment, one per line as KEY=value'),
          _LinesField(
            key: const ValueKey('agent-mcp-env'),
            controller: _mcp.env,
            placeholder: 'EXAMPLE_ROOT=/path',
          ),
        ],
        if (_mcp.editorError != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              _mcp.editorError!,
              key: const ValueKey('agent-mcp-editor-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ),
        const SizedBox(height: 8),
        Row(
          children: [
            SecondaryButton(
              label: _mcp.original == null ? 'Add' : 'Save',
              key: const ValueKey('agent-mcp-save'),
              height: 32,
              onTap: _mcp.save,
            ),
            const SizedBox(width: 8),
            TextBtn(
              'Cancel',
              key: const ValueKey('agent-mcp-cancel'),
              onTap: _mcp.close,
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
