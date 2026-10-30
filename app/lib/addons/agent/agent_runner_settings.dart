// Settings > Agent, shared by desktop and Android: "Where it runs" (the
// vault's synced runner: the hub or this computer, as two choice cards),
// each runner's model picker filled from that runner, and each runner's
// tool set ("What it may do": Read, Write or Everything). Everything else
// on the page depends on the runner chosen here.

import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../services/addon_services.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'agent_runner_choices.dart';
import 'agent_store.dart';

export 'agent_runner_choices.dart' show chosenRunner, runnerHelp;

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

/// "Where it runs": your hub or this computer, for every device of this
/// vault. On a phone "This computer" can't be picked.
class RunAiOnSetting extends StatelessWidget {
  const RunAiOnSetting({super.key, required this.store, this.onChanged, this.phone = false});
  final AgentStore store;
  final VoidCallback? onChanged;

  /// Phones always use the hub.
  final bool phone;

  @override
  Widget build(BuildContext context) {
    final chosen = phone ? AgentHost.hub : chosenRunner(store);
    return Padding(
      key: const ValueKey('agent-host'),
      padding: const EdgeInsets.only(top: 14),
      child: SChoices<AgentHost>(
        value: chosen,
        keyOf: (h) => ValueKey('agent-host-${h.runner}'),
        choices: [
          SChoice(AgentHost.hub, title: 'Your hub', desc: runnerHelp(AgentHost.hub), icon: Sx.server),
          SChoice(
            AgentHost.local,
            title: 'This computer',
            desc: runnerHelp(AgentHost.local),
            icon: Sx.monitor,
            disabledNote: phone ? 'Set this from Scratch on a computer. Phones always use the hub.' : null,
          ),
        ],
        onChanged: phone ? null : (h) => _write(context, () => store.setVaultDefaultHost(h), onChanged),
      ),
    );
  }
}

const _modelDesc = 'New chats start with this model. Each chat can switch.';
String _modelHelp(String who) =>
    'Pick a faster model for quick questions or a stronger one for long work. The list shows the models $who can use.';

/// The hub's model for new chats: "Hub default (...)" or one the hub lists.
/// Never typed: the list comes from the hub, or the one it last reported
/// when it can't be reached.
class HubModelSetting extends StatelessWidget {
  const HubModelSetting({super.key, required this.store, this.onChanged});
  final AgentStore store;
  final VoidCallback? onChanged;

  @override
  Widget build(BuildContext context) {
    final r = store.runnerModels(AgentHost.hub);
    final stored = store.vaultDefaults.models[AgentHost.hub];
    final ids = {for (final m in r.models) m.id}.toList();
    final missing = stored != null && r.live && !ids.contains(stored);
    String label(String? id) => id != null && id == stored && missing
        ? '${store.modelLabel(id)} (not available)'
        : hubModelLabel(id, r.hostDefault, store.modelLabel);
    return SRow(
      label: 'Model',
      desc: _modelDesc,
      help: _modelHelp('the hub'),
      stack: context.mobile,
      control: SSelect<String?>(
        key: const ValueKey('agent-hub-model'),
        title: 'Model',
        value: stored,
        options: hubModelOptions(ids, stored),
        labelOf: label,
        onChanged: (m) => _write(context, () => store.setVaultDefaultModel(AgentHost.hub, m), onChanged),
      ),
      below: r.unreachable
          ? SNotice(
              ids.isEmpty ? "Can't reach the hub." : "Can't reach the hub. These are the models it last listed.",
              kind: SNoticeKind.warn,
              textKey: const ValueKey('agent-hub-unreachable'),
            )
          : missing
          ? SNotice(
              'The saved model, ${store.modelLabel(stored)}, isn’t on the hub any more. New chats use the hub’s default until you pick another.',
              kind: SNoticeKind.warn,
              textKey: const ValueKey('agent-hub-model-missing'),
            )
          : null,
    );
  }
}

/// This computer's model for new chats and Ask AI, from its signed-in
/// providers (Codex, Claude Code). A saved model this computer no longer
/// lists is shown as missing, never replaced by the first one.
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
      return SRow(
        label: 'Model',
        desc: r.unreachable
            ? "Can't list this computer's models. Sign in to Codex or Claude Code on this computer."
            : "Reading this computer's models…",
        help: _modelHelp('this computer'),
        control: stored == null
            ? null
            : Text(
                store.modelLabel(stored),
                key: const ValueKey('agent-computer-model-stored'),
                style: tsRaw(13, 18, 400, c.textSecondary),
              ),
      );
    }
    final missing = stored != null && !ids.contains(stored);
    return SRow(
      label: 'Model',
      desc: _modelDesc,
      help: _modelHelp('this computer'),
      stack: context.mobile,
      control: SSelect<String>(
        key: const ValueKey('agent-computer-model'),
        title: 'Model',
        value: computerModelValue(ids, stored),
        options: ids,
        labelOf: (id) => id == stored && missing ? '${store.modelLabel(id)} (not available)' : store.modelLabel(id),
        onChanged: (m) => _write(context, () => store.setVaultDefaultModel(AgentHost.local, m), onChanged),
      ),
      below: missing
          ? SNotice(
              'The saved model, ${store.modelLabel(stored)}, isn’t available on this computer. Pick another.',
              kind: SNoticeKind.warn,
              textKey: const ValueKey('agent-computer-model-missing'),
            )
          : null,
    );
  }
}

/// [runner]'s tool set (synced) as three cards. The conversation tools come
/// with every set; files, commands and MCP servers only with Everything.
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
    final computer = !onHub && !context.mobile;
    String title(String s) => switch (s) {
      'read' => 'Read',
      'write' => 'Write',
      _ => toolSetLabel(s),
    };
    String desc(String s) => switch (s) {
      'read' => 'Search and read your pages.',
      'write' => 'Read, plus create and edit pages.',
      _ => computer
          ? 'Every vault tool, plus computer access below and MCP servers on Connectors.'
          : 'Every vault tool.',
    };
    return Column(
      key: ValueKey('agent-tool-set-${runner.runner}'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 14),
        SChoices<String>(
          value: set,
          keyOf: (s) => ValueKey('agent-tool-set-${runner.runner}-$s'),
          choices: [for (final s in toolSets) SChoice(s, title: title(s), desc: desc(s))],
          onChanged: (v) => _write(context, () => store.setVaultToolSet(runner, v), onChanged),
        ),
        if (onHub)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              'Jobs use it at once; hub chats after the hub restarts (an update or reboot).',
              style: sHint(context),
            ),
          ),
      ],
    );
  }
}
