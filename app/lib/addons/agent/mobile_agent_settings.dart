import '../../services/addon_services.dart';
// Settings > AI > Agent on Android: the agent runs on the Hub only, so the
// section is the Hub sign-in (address, Sign in or Sign out, the account),
// the sync connection, the agent's own connection with Retry, Sync now, and
// the hub's model and tool set. "Run AI on" is the vault's (synced); with
// This computer chosen, a line says phones use the hub. Desktop keeps
// AgentSettingsSection.
//
// Signed out, nothing here reaches the Hub: the agent and model rows are
// hidden. The agent row reports only what the store knows: it is connected
// when the agent is open, failed when opening failed, and not connected yet
// otherwise. A stored token alone is never shown as a connection.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../../ui/ui.dart';
import 'agent_runner_settings.dart';
import 'agent_store.dart';

/// The sync line: what the phone last knew about the Hub's sync.
String hubConnectionLine(SyncStore sync, {required bool signedIn}) {
  if (!signedIn) return 'Not connected. Sign in to the Hub to use the agent.';
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
  const MobileAgentSettings({super.key, required this.store, this.auth});
  final AgentStore store;

  /// Supplies a browser login in tests (as for [HubSignInSettings]).
  final HubAuthService? auth;

  @override
  Widget build(BuildContext context) {
    final app = context.app;
    return ListenableBuilder(
      listenable: Listenable.merge([app.settings, app.sync, store]),
      builder: (context, _) {
        final c = context.colors;
        // Signed in = the device holds a Hub token and address. Sync and the
        // agent then try to connect; neither is proof of a connection.
        final s = app.settings;
        final signedIn = s.clientToken != null && s.clientUrl != null;
        final chosen = chosenRunner(store);
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            RunAiOnSetting(store: store),
            if (chosen == AgentHost.local)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: Text(
                  'Phones use the hub.',
                  key: const ValueKey('agent-phone-uses-hub'),
                  style: tsRaw(13, 18, 400, c.textSecondary),
                ),
              ),
            HubSignInSettings(auth: auth),
            SettingsRow(
              label: 'Sync',
              description: hubConnectionLine(app.sync, signedIn: signedIn),
              trailing: signedIn
                  ? SecondaryButton(
                      key: const ValueKey('hub-sync-now'),
                      label: 'Sync now',
                      height: 44,
                      onTap: app.sync.connected ? app.sync.syncNow : null,
                    )
                  : null,
            ),
            if (signedIn)
              SettingsRow(
                label: 'Agent',
                description: agentConnectionLine(store),
                trailing: store.openError != null && !store.isOpen
                    ? SecondaryButton(
                        key: const ValueKey('agent-retry-connection'),
                        label: 'Retry connection',
                        height: 44,
                        onTap: () => store.ensureOpen(),
                      )
                    : null,
              ),
            if (signedIn && store.openError != null)
              Text(
                store.openError!,
                key: const ValueKey('agent-settings-error'),
                style: tsRaw(12, 16, 400, c.inlineCodeText),
              ),
            if (signedIn) ...[
              HubModelSetting(store: store),
              RunnerToolSetSetting(store: store, runner: AgentHost.hub),
            ],
          ],
        );
      },
    );
  }
}
