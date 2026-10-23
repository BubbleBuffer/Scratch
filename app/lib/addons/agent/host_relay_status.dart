// Under "Let the hub use this computer" (Settings > Sync, desktop) while it
// is on: the name this computer has on the hub, the relay's connection state
// (read every [HostRelayLink.statusInterval] while shown) and the connector
// address for ChatGPT or claude.ai, with Copy, and what it shares: this
// computer's tools and MCP servers, with a link to the MCP servers (shown in
// Settings > AI whichever runner the vault uses while this is on).
//
// The name is the hub's label for this device (`clientDeviceLabel`), from
// sign-in or a Rename here; the hub matches `?host=` against it (exactly,
// else ignoring case). A rename changes the address at once and the old one
// stops working; the hub refuses a name another computer already has.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_mcp_settings.dart';
import 'agent_store.dart';
import 'host_relay_link.dart';

/// What the hub may use on this computer, in one line.
String hostRelayToolsLine(AgentStore store) {
  final set = store.vaultDefaults.toolSetFor(AgentHost.local);
  if (set != 'everything') {
    return "Shares this computer's vault tools (${toolSetLabel(set)}).";
  }
  final n = store.settings.agentMcpServers.values
      .where((v) => v['enabled'] != false)
      .length;
  final tools = store.settings.desktopTools
      ? 'The vault, files and commands.'
      : 'The vault.';
  return switch (n) {
    0 => tools,
    1 => '$tools Plus 1 MCP server.',
    _ => '$tools Plus $n MCP servers.',
  };
}

/// Opens Settings > AI at the MCP servers.
Future<void> showMcpServers(BuildContext context) async {
  await openSettingsSection(context, SettingsSection.ai);
  SchedulerBinding.instance.addPostFrameCallback((_) {
    final target = mcpServersAnchor.currentContext;
    if (target != null && target.mounted) Scrollable.ensureVisible(target);
  });
}

/// `<hub>/mcp/agent?host=<name>`: the connector address pinned to this
/// computer, through [hubUrl].
String hostConnectorUrl(String hubUrl, String name) =>
    '${anyComputerConnectorUrl(hubUrl)}?host=${Uri.encodeComponent(name.trim())}';

/// `<hub>/mcp/agent`: the connector address that reaches any of the user's
/// computers (each call names the computer in its `host` argument).
String anyComputerConnectorUrl(String hubUrl) {
  var base = hubUrl.trim();
  while (base.endsWith('/')) {
    base = base.substring(0, base.length - 1);
  }
  return '$base/mcp/agent';
}

class HostRelayDetails extends StatefulWidget {
  const HostRelayDetails({
    super.key,
    required this.link,
    required this.hubUrl,
    required this.name,
    this.store,
    this.onRename,
  });

  final HostRelayLink link;

  /// For the line of what it shares (null: not shown).
  final AgentStore? store;

  /// The signed-in hub's address and this computer's name on it.
  final String hubUrl, name;

  /// Renames this computer on the hub (null: no Rename). Throws with a
  /// message to show when the hub refuses the name.
  final Future<void> Function(String name)? onRename;

  @override
  State<HostRelayDetails> createState() => _HostRelayDetailsState();
}

class _HostRelayDetailsState extends State<HostRelayDetails> {
  final _rename = TextEditingController();
  bool _renaming = false, _saving = false;
  String? _renameError;

  @override
  void initState() {
    super.initState();
    widget.link.watchStatus();
  }

  @override
  void didUpdateWidget(HostRelayDetails old) {
    super.didUpdateWidget(old);
    if (!identical(old.link, widget.link)) {
      old.link.unwatchStatus();
      widget.link.watchStatus();
    }
  }

  @override
  void dispose() {
    widget.link.unwatchStatus();
    _rename.dispose();
    super.dispose();
  }

  Future<void> _saveName() async {
    final rename = widget.onRename;
    if (rename == null || _saving) return;
    final name = _rename.text.trim();
    if (name.isEmpty) {
      setState(() => _renameError = 'Enter a name.');
      return;
    }
    setState(() {
      _saving = true;
      _renameError = null;
    });
    try {
      await rename(name);
      if (!mounted) return;
      setState(() {
        _saving = false;
        _renaming = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _renameError = '$e';
      });
    }
  }

  Widget? _nameTrailing(AppColors c) {
    if (!_renaming) {
      if (widget.onRename == null) return null;
      return TextBtn(
        'Rename',
        key: const ValueKey('host-relay-rename'),
        onTap: () => setState(() {
          _renaming = true;
          _renameError = null;
          _rename.text = widget.name;
        }),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: SettingsField(
                fieldKey: const ValueKey('host-relay-rename-field'),
                controller: _rename,
                autofocus: true,
                onSubmitted: (_) => _saveName(),
              ),
            ),
            const SizedBox(width: 8),
            TextBtn(
              'Save',
              key: const ValueKey('host-relay-rename-save'),
              onTap: _saving ? null : _saveName,
            ),
            TextBtn(
              'Cancel',
              key: const ValueKey('host-relay-rename-cancel'),
              onTap: () => setState(() {
                _renaming = false;
                _renameError = null;
              }),
            ),
          ],
        ),
        if (_renameError case final error?)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              error,
              key: const ValueKey('host-relay-rename-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ),
      ],
    );
  }

  /// The state in words, and whether it is a problem.
  (String, bool) _describe(AgentHostRelayStatus? s) {
    final error = s?.error?.trim();
    final why = error == null || error.isEmpty ? null : error;
    return switch (s?.state) {
      null => ('Checking the connection…', false),
      'connected' => ('Connected to the hub.', false),
      'connecting' => ('Connecting to the hub…', false),
      'retrying' => (
        why == null
            ? "Can't reach the hub. Trying again."
            : "Can't reach the hub: $why. Trying again.",
        true,
      ),
      _ when why != null => ('Not connected: $why', true),
      _ when !widget.hubUrl.trim().toLowerCase().startsWith('https://') => (
        'Not connected. The hub needs an https:// address.',
        true,
      ),
      _ => ('Not connected.', false),
    };
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          key: const ValueKey('host-relay-name'),
          label: _renaming
              ? 'Name on the hub'
              : 'Appears on the hub as ${widget.name}',
          description:
              'The name AI apps use for this computer. Renaming it changes '
              'the address below; the old address stops working.',
          stack: _renaming,
          trailing: _nameTrailing(c),
        ),
        ValueListenableBuilder<AgentHostRelayStatus?>(
          valueListenable: widget.link.status,
          builder: (context, status, _) {
            final (text, problem) = _describe(status);
            return Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                text,
                key: const ValueKey('host-relay-status'),
                style: tsRaw(
                  12,
                  16,
                  400,
                  problem ? c.inlineCodeText : c.textSecondary,
                ),
              ),
            );
          },
        ),
        if (widget.store case final store?)
          ListenableBuilder(
            listenable: store,
            builder: (context, _) => SettingsRow(
              key: const ValueKey('host-relay-tools'),
              label: 'What it shares',
              description: hostRelayToolsLine(store),
              trailing: OutlineButton(
                key: const ValueKey('host-relay-mcp-link'),
                label: 'Edit MCP servers',
                onTap: () => showMcpServers(context),
              ),
            ),
          ),
        // Two connector addresses for ChatGPT or claude.ai: pinned to this
        // computer, or any computer with the relay on (the assistant picks).
        SettingsRow(
          label: 'This computer only',
          description:
              'Add it as a custom connector in ChatGPT or claude.ai to use '
              'this computer through the hub.',
          stack: true,
          trailing: CopyableText(
            hostConnectorUrl(widget.hubUrl, widget.name),
            key: const ValueKey('host-relay-url'),
            textKey: const ValueKey('host-relay-url-text'),
          ),
        ),
        SettingsRow(
          label: 'Any of your computers',
          description:
              'One connector for every computer with this setting on; the AI '
              'picks which one to use.',
          stack: true,
          trailing: CopyableText(
            anyComputerConnectorUrl(widget.hubUrl),
            key: const ValueKey('host-relay-any-url'),
            textKey: const ValueKey('host-relay-any-url-text'),
          ),
        ),
      ],
    );
  }
}
