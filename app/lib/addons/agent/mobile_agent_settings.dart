import '../../services/addon_services.dart';
// Settings > AI > Agent on Android: the agent runs on the Hub only, so the
// section is the Hub sign-in (address, Sign in or Sign out, the account),
// the sync connection, the agent's own connection with Retry, Sync now and
// the default model when the Hub exposes models. Desktop keeps
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
        final models = store.hubLink != null
            ? store.shownModels()
            : const <String>[];
        // The vault's default model for the Hub (synced), and this phone's
        // override of it.
        final current = store.vaultDefaults.models[AgentHost.hub] ?? '';
        final override = store.settings.agentModelOverrides[AgentHost.hub];
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SettingsRow(
              label: 'Hub',
              description: 'The agent runs on your notes hub, with its own models and sign-ins.',
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
            if (signedIn && store.hostDescription != null)
              Text(
                store.hostDescription!,
                key: const ValueKey('agent-host-info'),
                style: tsRaw(12, 16, 400, c.textSecondary),
              ),
            if (models.isNotEmpty) ...[
              SettingsRow(
                label: 'Default model',
                description: 'New chats on the hub use it, on every device of this vault. Each chat can pick another.',
                stack: true,
                trailing: SelectButton<String>(
                  key: const ValueKey('agent-default-model'),
                  title: 'Default model',
                  value: models.contains(current) ? current : models.first,
                  options: models,
                  labelOf: store.modelLabel,
                  onChanged: (m) =>
                      store.setVaultDefaultModel(AgentHost.hub, m),
                ),
              ),
              SettingsRow(
                label: 'Model on this device',
                description: override == null
                    ? 'Follows the vault setting.'
                    : 'Overrides the vault setting on this device only.',
                stack: true,
                trailing: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SelectButton<String?>(
                      key: const ValueKey('agent-model-override'),
                      title: 'Model on this device',
                      value: override,
                      options: [
                        ...models,
                        if (override != null && !models.contains(override))
                          override,
                      ],
                      labelOf: (m) =>
                          m == null ? 'Vault setting' : store.modelLabel(m),
                      onChanged: (m) => store.setModelOverride(AgentHost.hub, m),
                    ),
                    if (override != null) ...[
                      const SizedBox(width: 8),
                      SecondaryButton(
                        key: const ValueKey('agent-model-override-clear'),
                        label: 'Clear',
                        height: 44,
                        onTap: () => store.setModelOverride(AgentHost.hub, null),
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ],
        );
      },
    );
  }
}
