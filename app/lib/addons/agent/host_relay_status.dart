// Settings > Connectors > "This computer for hub apps" while "Let the hub
// use this computer" is on: the name this computer has on the hub, the
// relay's connection state (read every [HostRelayLink.statusInterval] while
// shown), what it shares (this computer's tools and MCP servers, with a link
// to the MCP servers panel) and the connector addresses for ChatGPT or
// claude.ai, with Copy.
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

/// Opens Settings > Connectors; with [open], with "Tools for the agent"
/// open and scrolled to the MCP servers.
Future<void> showMcpServers(BuildContext context, {bool open = true}) async {
  if (open) connectorPanels.value = {...connectorPanels.value, 'tools'};
  await openSettingsSection(context, SettingsSection.connectors);
  if (!open) return;
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
    if (_renaming || widget.onRename == null) return null;
    return SButton(
      'Rename',
      key: const ValueKey('host-relay-rename'),
      onTap: () => setState(() {
        _renaming = true;
        _renameError = null;
        _rename.text = widget.name;
      }),
    );
  }

  Widget _renameEditor() => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Row(
        children: [
          Expanded(
            child: STextField(
              fieldKey: const ValueKey('host-relay-rename-field'),
              controller: _rename,
              autofocus: true,
              onSubmitted: (_) => _saveName(),
            ),
          ),
          const SizedBox(width: 8),
          SButton(
            'Cancel',
            key: const ValueKey('host-relay-rename-cancel'),
            kind: SButtonKind.quiet,
            onTap: () => setState(() {
              _renaming = false;
              _renameError = null;
            }),
          ),
          const SizedBox(width: 4),
          SButton(
            'Save',
            key: const ValueKey('host-relay-rename-save'),
            kind: SButtonKind.primary,
            onTap: _saving ? null : _saveName,
          ),
        ],
      ),
      if (_renameError case final error?)
        SNotice(
          error,
          kind: SNoticeKind.bad,
          textKey: const ValueKey('host-relay-rename-error'),
        ),
    ],
  );

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
        SRow(
          key: const ValueKey('host-relay-name'),
          label: _renaming
              ? 'Name on the hub'
              : 'Appears on the hub as ${widget.name}',
          desc:
              'The name AI apps use for this computer. Renaming it changes '
              'the addresses below; the old ones stop working.',
          control: _nameTrailing(c),
          below: _renaming ? _renameEditor() : null,
        ),
        ValueListenableBuilder<AgentHostRelayStatus?>(
          valueListenable: widget.link.status,
          builder: (context, status, _) {
            final (text, problem) = _describe(status);
            return SRow(
              label: 'Connection',
              desc: text,
              control: SStatus(
                status?.state == 'connected'
                    ? 'Connected'
                    : problem
                    ? 'Problem'
                    : 'Not connected',
                key: const ValueKey('host-relay-status'),
                kind: status?.state == 'connected'
                    ? SStatusKind.ok
                    : problem
                    ? SStatusKind.bad
                    : SStatusKind.off,
              ),
            );
          },
        ),
        if (widget.store case final store?)
          ListenableBuilder(
            listenable: store,
            builder: (context, _) => SRow(
              key: const ValueKey('host-relay-tools'),
              label: 'What it shares',
              desc: hostRelayToolsLine(store),
              control: SButton(
                'Edit MCP servers',
                key: const ValueKey('host-relay-mcp-link'),
                onTap: () => showMcpServers(context),
              ),
            ),
          ),
        // Two connector addresses for ChatGPT or claude.ai: pinned to this
        // computer, or any computer with the relay on (the assistant picks).
        SRow(
          label: 'This computer only',
          desc:
              'Add it as a custom connector in ChatGPT or claude.ai to use '
              'this computer through the hub.',
          below: SCopyLine(
            hostConnectorUrl(widget.hubUrl, widget.name),
            key: const ValueKey('host-relay-url'),
            textKey: const ValueKey('host-relay-url-text'),
          ),
        ),
        SRow(
          label: 'Any of your computers',
          desc:
              'One connector for every computer with this setting on; the AI '
              'picks which one to use.',
          below: SCopyLine(
            anyComputerConnectorUrl(widget.hubUrl),
            key: const ValueKey('host-relay-any-url'),
            textKey: const ValueKey('host-relay-any-url-text'),
          ),
        ),
      ],
    );
  }
}
