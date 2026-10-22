// Settings > AI > Agent, shared by desktop and Android: "Run AI on" (the
// vault's synced runner: the hub or this computer), each runner's model
// picker filled from that runner, and each runner's tool set. Everything
// else in the section depends on the runner chosen here.

import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../services/addon_services.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';

/// What each runner does, in one line.
String runnerHelp(AgentHost h) => switch (h) {
  AgentHost.hub => 'Your hub runs the AI; works from every device.',
  AgentHost.local =>
    'This computer runs the AI with its own models and tools.',
};

/// The runner chosen for the vault (synced), or the built-in default.
AgentHost chosenRunner(AgentStore store) =>
    store.vaultDefaults.host ?? builtInAgentHost;

Future<void> _write(
  BuildContext context,
  Future<void> Function() f, [
  VoidCallback? onChanged,
]) async {
  try {
    await f();
  } catch (e) {
    if (context.mounted) {
      showErrorToast(context, 'Could not change this setting. Try again.');
    }
  }
  onChanged?.call();
}

/// "Run AI on": Hub or This computer, for every device of this vault.
class RunAiOnSetting extends StatelessWidget {
  const RunAiOnSetting({super.key, required this.store, this.onChanged});
  final AgentStore store;
  final VoidCallback? onChanged;

  @override
  Widget build(BuildContext context) {
    final chosen = chosenRunner(store);
    return SettingsRow(
      label: 'Run AI on',
      description: runnerHelp(chosen),
      trailing: SelectButton<AgentHost>(
        key: const ValueKey('agent-host'),
        title: 'Run AI on',
        value: chosen,
        options: const [AgentHost.hub, AgentHost.local],
        labelOf: (h) => h.label,
        onChanged: (h) =>
            _write(context, () => store.setVaultDefaultHost(h), onChanged),
      ),
    );
  }
}

/// The hub's model for new chats: "Hub default (...)" or one the hub lists.
/// Never typed: the list comes from the hub, or the one it last reported
/// when it can't be reached.
class HubModelSetting extends StatelessWidget {
  const HubModelSetting({super.key, required this.store, this.onChanged});
  final AgentStore store;
  final VoidCallback? onChanged;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final r = store.runnerModels(AgentHost.hub);
    final stored = store.vaultDefaults.models[AgentHost.hub];
    final ids = {for (final m in r.models) m.id}.toList();
    final hostDefault = r.hostDefault;
    String label(String? id) => id == null
        ? hostDefault == null
              ? 'Hub default'
              : 'Hub default (${store.modelLabel(hostDefault)})'
        : store.modelLabel(id);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          label: 'Model',
          description: 'For new chats on every device. Each chat can pick another.',
          stack: context.mobile,
          trailing: SelectButton<String?>(
            key: const ValueKey('agent-hub-model'),
            title: 'Model',
            value: stored,
            options: [
              null,
              ...ids,
              if (stored != null && !ids.contains(stored)) stored,
            ],
            labelOf: label,
            onChanged: (m) => _write(
              context,
              () => store.setVaultDefaultModel(AgentHost.hub, m),
              onChanged,
            ),
          ),
        ),
        if (r.unreachable)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              ids.isEmpty
                  ? "Can't reach the hub."
                  : "Can't reach the hub. These are the models it last listed.",
              key: const ValueKey('agent-hub-unreachable'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          ),
      ],
    );
  }
}

/// This computer's model for new chats and Ask AI, from its signed-in
/// providers (Codex, Claude Code).
class ComputerModelSetting extends StatelessWidget {
  const ComputerModelSetting({super.key, required this.store, this.onChanged});
  final AgentStore store;
  final VoidCallback? onChanged;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final r = store.runnerModels(AgentHost.local);
    final stored = store.vaultDefaults.models[AgentHost.local];
    final ids = {for (final m in r.models) m.id}.toList();
    if (ids.isEmpty) {
      return SettingsRow(
        label: 'Model',
        description: r.unreachable
            ? "Can't list this computer's models. Sign in to Codex or Claude Code on this computer."
            : "Reading this computer's models…",
        trailing: stored == null
            ? null
            : Text(
                store.modelLabel(stored),
                key: const ValueKey('agent-computer-model-stored'),
                style: tsRaw(13, 18, 400, c.textSecondary),
              ),
      );
    }
    return SettingsRow(
      label: 'Model',
      description: 'For new chats and Ask AI. Each chat can pick another.',
      trailing: SelectButton<String>(
        key: const ValueKey('agent-computer-model'),
        title: 'Model',
        value: stored != null && ids.contains(stored) ? stored : ids.first,
        options: ids,
        labelOf: store.modelLabel,
        onChanged: (m) => _write(
          context,
          () => store.setVaultDefaultModel(AgentHost.local, m),
          onChanged,
        ),
      ),
    );
  }
}

/// [runner]'s tool set (synced). The conversation tools come with every
/// set; files, commands and MCP servers only with Everything.
class RunnerToolSetSetting extends StatelessWidget {
  const RunnerToolSetSetting({
    super.key,
    required this.store,
    required this.runner,
    this.onChanged,
  });
  final AgentStore store;
  final AgentHost runner;
  final VoidCallback? onChanged;

  @override
  Widget build(BuildContext context) {
    final set = store.vaultDefaults.toolSetFor(runner);
    final onHub = runner == AgentHost.hub;
    final what = switch (set) {
      'read' => 'Reads the vault and changes nothing.',
      'write' => 'Reads and edits the vault.',
      _ =>
        onHub
            ? 'Every tool the hub has.'
            : 'The vault, plus files, commands and MCP servers here.',
    };
    return SettingsRow(
      label: 'Tools',
      description: onHub ? '$what Applies when the hub restarts.' : what,
      trailing: SelectButton<String>(
        key: ValueKey('agent-tool-set-${runner.runner}'),
        title: 'Tools',
        value: set,
        options: toolSets,
        labelOf: toolSetLabel,
        onChanged: (v) =>
            _write(context, () => store.setVaultToolSet(runner, v), onChanged),
      ),
    );
  }
}
