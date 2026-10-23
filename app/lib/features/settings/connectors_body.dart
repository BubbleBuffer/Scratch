// Settings > Connectors: three panels, closed by default, each with a status
// in its header.
//
// - Tools for the agent: the MCP servers the agent on this computer starts
//   (desktop). Locked unless this computer's tool set is Everything.
// - Apps that use your vault: quick setup commands (desktop) and the hub's
//   access tokens (every device signed in to a hub).
// - This computer for hub apps: "Let the hub use this computer" (desktop,
//   signed in), then its name on the hub, connection and addresses.
//
// Which panels are open is [connectorPanels], so other pages can open one
// before jumping here ([showMcpServers]); it is cleared when this page goes.

import 'package:flutter/widgets.dart';

import '../../addons/agent/api.dart'
    show
        AgentStore,
        HostRelayDetails,
        McpServersSettings,
        mcpServersAnchor,
        mcpServersSummary;
import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../ai/agent_tokens_section.dart';
import '../ai/ai_settings.dart';
import '../ai/connect_agent_section.dart';
import '../sync/hub_device_client.dart';
import '../../ui/ui.dart';
import 'settings_kit.dart';
import 'settings_screen.dart';
import 'settings_store.dart';

const _onComputer = 'Set these from Scratch on a computer. Phones always use the hub.';

class ConnectorsBody extends StatefulWidget {
  const ConnectorsBody({super.key});

  @override
  State<ConnectorsBody> createState() => _ConnectorsBodyState();
}

class _ConnectorsBodyState extends State<ConnectorsBody> {
  int? _tokens;

  @override
  void dispose() {
    connectorPanels.value = <String>{};
    super.dispose();
  }

  void _toggle(String id) {
    final open = {...connectorPanels.value};
    if (!open.remove(id)) open.add(id);
    connectorPanels.value = open;
  }

  @override
  Widget build(BuildContext context) {
    final app = context.app;
    final agent = AgentStore.of(app);
    return ListenableBuilder(
      listenable: Listenable.merge([connectorPanels, app.settings, agent]),
      builder: (context, _) {
        final open = connectorPanels.value;
        final s = app.settings;
        final signedIn = s.clientToken != null && s.clientUrl != null;
        // MCP servers and the relay belong to a computer that runs the agent.
        final computer =
            AgentStore.canRun && localAgentPlatform && !hubAgentPlatform;
        final everything =
            agent.vaultDefaults.toolSetFor(AgentHost.local) == 'everything';
        final toolsStatus = !computer
            ? 'On a computer'
            : !everything
            ? 'Off'
            : mcpServersSummary(agent);
        final appsStatus = !signedIn
            ? 'Needs a hub'
            : switch (_tokens) {
                null => 'Signed in',
                0 => 'No tokens',
                1 => '1 token',
                final n => '$n tokens',
              };
        final hubStatus = !computer
            ? 'On a computer'
            : !signedIn
            ? 'Needs a hub'
            : s.hubUsesThisComputer
            ? 'On'
            : 'Off';
        return SPage(
          title: 'Connectors',
          intro:
              'Connectors link Scratch with other tools, both ways. Scratch’s agent can use outside tools, and outside AI apps can use your vault.',
          children: [
            const SizedBox(height: 10),
            SPanel(
              headerKey: const ValueKey('connectors-tools'),
              icon: Sx.wrench,
              title: 'Tools for the agent',
              desc:
                  'Extra tools the agent can use, from programs on this computer or web addresses.',
              status: toolsStatus,
              open: open.contains('tools'),
              onToggle: () => _toggle('tools'),
              child: computer
                  ? KeyedSubtree(
                      key: mcpServersAnchor,
                      child: McpServersSettings(store: agent),
                    )
                  : Padding(
                      padding: const EdgeInsets.only(top: 12),
                      child: Text(_onComputer, style: sDesc(context)),
                    ),
            ),
            SPanel(
              headerKey: const ValueKey('connectors-apps'),
              icon: Sx.appWindow,
              title: 'Apps that use your vault',
              desc:
                  'Let AI apps such as Claude Code or Codex read and edit your vault.',
              status: appsStatus,
              open: open.contains('apps'),
              onToggle: () => _toggle('apps'),
              keepAlive: true,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  // Its `notes setup` commands run in a desktop terminal.
                  if (!context.mobile) const ConnectAgentSection(first: true),
                  AgentTokensSection(
                    first: context.mobile,
                    onCount: (n) {
                      if (mounted && n != _tokens) setState(() => _tokens = n);
                    },
                  ),
                ],
              ),
            ),
            SPanel(
              headerKey: const ValueKey('connectors-hub'),
              icon: Sx.monitor,
              title: 'This computer for hub apps',
              desc:
                  'Let AI apps on your other devices work on this computer’s files.',
              status: hubStatus,
              open: open.contains('hub'),
              onToggle: () => _toggle('hub'),
              child: computer
                  ? _HubPanel(signedIn: signedIn, agent: agent)
                  : Padding(
                      padding: const EdgeInsets.only(top: 12),
                      child: Text(_onComputer, style: sDesc(context)),
                    ),
            ),
          ],
        );
      },
    );
  }
}

/// "Let the hub use this computer", then while on: this computer's name on
/// the hub, the connection and the connector addresses.
class _HubPanel extends StatelessWidget {
  const _HubPanel({required this.signedIn, required this.agent});
  final bool signedIn;
  final AgentStore agent;

  @override
  Widget build(BuildContext context) {
    final s = context.app.settings;
    final on = signedIn && s.hubUsesThisComputer;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SRow(
          label: 'Let the hub use this computer',
          desc:
              'AI apps connected to your hub can read and edit files and run commands on this computer while Scratch is open. Off by default.',
          help:
              'Turn on to let an AI app on your phone or another machine work on this computer’s files. Leave off otherwise.',
          disabled: !signedIn,
          reason: signedIn ? null : 'Sign in to a hub first.',
          reasonActionLabel: signedIn ? null : 'Open Sync and devices',
          reasonAction: signedIn
              ? null
              : () => openSettingsSection(context, SettingsSection.sync),
          control: SSwitch(
            key: const ValueKey('host-relay-toggle'),
            label: 'Let the hub use this computer',
            value: s.hubUsesThisComputer,
            onChanged: signedIn
                ? (v) => AppSettings.read(
                    context,
                  ).update((s) => s.hubUsesThisComputer = v)
                : null,
          ),
        ),
        if (on)
          HostRelayDetails(
            link: agent.relay,
            store: agent,
            hubUrl: s.clientUrl!,
            name: (s.clientDeviceLabel ?? '').trim(),
            onRename: (name) async {
              final settings = AppSettings.read(context);
              final deviceId = context.read<SyncStore>().vault.deviceId;
              final used = await renameHubDevice(
                hubUrl: s.clientUrl!,
                token: s.clientToken ?? '',
                deviceId: deviceId,
                name: name,
              );
              settings.update((s) => s.clientDeviceLabel = used);
            },
          ),
      ],
    );
  }
}
