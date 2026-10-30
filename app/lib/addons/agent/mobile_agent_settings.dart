// Settings > Agent on Android: the agent runs on the hub only. "Where it
// runs" shows the hub (This computer can't be picked on a phone; when the
// vault's choice is This computer, a line says this phone still uses the
// hub), the agent's connection with Retry, and the hub's model and tool
// set. Hub sign-in and sync live on Settings > Sync and devices; signed
// out, a notice links there. Desktop keeps AgentSettingsSection.
//
// Signed out, nothing here reaches the hub: the model and tool set are
// hidden. The connection row reports only what the store knows: connected
// when the agent is open, failed when opening failed, and not connected yet
// otherwise. A stored token alone is never shown as a connection.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'agent_runner_settings.dart';
import 'agent_store.dart';

/// The sync line: what the phone last knew about the Hub's sync.
String hubConnectionLine(SyncStore sync, {required bool signedIn}) {
  if (!signedIn) {
    return 'Not connected. Sign in to your hub under Sync and devices to use the agent.';
  }
  if (!sync.connected) return 'Not connected to the Hub right now.';
  final st = sync.status;
  final error = st.lastError;
  if (error != null && error.isNotEmpty) return 'Last sync failed: $error';
  if (st.syncing) return 'Syncing now.';
  final ok = st.lastOkMs;
  if (ok != null) return 'Connected. Last synced ${relativeTime(ok.toInt())}.';
  return 'Connected. Waiting for the first sync.';
}

/// The agent line, for a signed-in device: from the store's open state.
String agentConnectionLine(AgentStore store) {
  if (store.isOpen) return 'Agent connected. It runs on the Hub.';
  final error = store.openError;
  if (error != null) return 'Agent could not connect: $error';
  return 'Agent not connected yet.';
}

class MobileAgentSettings extends StatelessWidget {
  const MobileAgentSettings({super.key, required this.store});
  final AgentStore store;

  @override
  Widget build(BuildContext context) {
    final app = context.app;
    return ListenableBuilder(
      listenable: Listenable.merge([app.settings, app.sync, store]),
      builder: (context, _) {
        // Signed in = the device holds a hub token and address. Sync and the
        // agent then try to connect; neither is proof of a connection.
        final s = app.settings;
        final signedIn = s.clientToken != null && s.clientUrl != null;
        final chosen = chosenRunner(store);
        final failed = store.openError != null && !store.isOpen;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SSection(
              first: true,
              title: 'Where it runs',
              children: [
                RunAiOnSetting(store: store, phone: true),
                if (chosen == AgentHost.local)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(
                      'Your computers use their own agent. This phone uses the hub.',
                      key: const ValueKey('agent-phone-uses-hub'),
                      style: sHint(context),
                    ),
                  ),
                if (!signedIn)
                  SNotice(
                    hubConnectionLine(app.sync, signedIn: false),
                    kind: SNoticeKind.warn,
                    textKey: const ValueKey('agent-signed-out'),
                    actions: [
                      SButton(
                        'Open Sync and devices',
                        key: const ValueKey('agent-open-sync'),
                        onTap: () =>
                            openSettingsSection(context, SettingsSection.sync),
                      ),
                    ],
                  )
                else ...[
                  SRow(
                    label: 'Connection',
                    desc: agentConnectionLine(store),
                    control: SStatus(
                      store.isOpen
                          ? 'Connected'
                          : failed
                          ? 'Problem'
                          : 'Not connected',
                      key: const ValueKey('agent-connection-status'),
                      kind: store.isOpen
                          ? SStatusKind.ok
                          : failed
                          ? SStatusKind.bad
                          : SStatusKind.off,
                    ),
                    below: failed
                        ? Padding(
                            padding: const EdgeInsets.only(top: 10),
                            child: Align(
                              alignment: Alignment.centerLeft,
                              child: SButton(
                                'Retry connection',
                                key: const ValueKey('agent-retry-connection'),
                                onTap: () => store.ensureOpen(),
                              ),
                            ),
                          )
                        : null,
                  ),
                  HubModelSetting(store: store),
                ],
              ],
            ),
            if (signedIn)
              SSection(
                title: 'What it may do',
                intro: 'Applies to the agent on your hub.',
                children: [
                  RunnerToolSetSetting(store: store, runner: AgentHost.hub),
                ],
              ),
          ],
        );
      },
    );
  }
}
