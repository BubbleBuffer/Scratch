// Under "Let the hub use this computer" (Settings > Sync, desktop) while it
// is on: the name this computer has on the hub, the relay's connection state
// (read every [HostRelayLink.statusInterval] while shown) and the connector
// address for ChatGPT or claude.ai, with Copy, and what it shares: this
// computer's tools and MCP servers, with a link to the MCP servers (shown in
// Settings > AI whichever runner the vault uses while this is on).
//
// The name is the hub's label for this device, from sign-in
// (`clientDeviceLabel`); the hub matches `?host=` against it exactly.

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
  final servers = switch (n) {
    0 => 'no MCP servers',
    1 => '1 MCP server',
    _ => '$n MCP servers',
  };
  final files = store.settings.desktopTools ? ', files, commands' : '';
  return "Shares this computer's tools: the vault$files and $servers.";
}

/// Opens Settings > AI at the MCP servers.
Future<void> showMcpServers(BuildContext context) async {
  await openSettingsSection(context, SettingsSection.ai);
  SchedulerBinding.instance.addPostFrameCallback((_) {
    final target = mcpServersAnchor.currentContext;
    if (target != null && target.mounted) Scrollable.ensureVisible(target);
  });
}

/// `<hub>/mcp/agent?host=<name>`: the connector address that runs this
/// computer's tools through [hubUrl].
String hostConnectorUrl(String hubUrl, String name) {
  var base = hubUrl.trim();
  while (base.endsWith('/')) {
    base = base.substring(0, base.length - 1);
  }
  return '$base/mcp/agent?host=${Uri.encodeComponent(name.trim())}';
}

class HostRelayDetails extends StatefulWidget {
  const HostRelayDetails({
    super.key,
    required this.link,
    required this.hubUrl,
    required this.name,
    this.store,
  });

  final HostRelayLink link;

  /// For the line of what it shares (null: not shown).
  final AgentStore? store;

  /// The signed-in hub's address and this computer's name on it.
  final String hubUrl, name;

  @override
  State<HostRelayDetails> createState() => _HostRelayDetailsState();
}

class _HostRelayDetailsState extends State<HostRelayDetails> {
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
    super.dispose();
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
          label: 'Appears on the hub as ${widget.name}',
          description:
              "This computer's name from when it signed in to the hub.",
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
              trailing: TextBtn(
                'MCP servers',
                key: const ValueKey('host-relay-mcp-link'),
                onTap: () => showMcpServers(context),
              ),
            ),
          ),
        SettingsRow(
          label: 'Connector address',
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
      ],
    );
  }
}
