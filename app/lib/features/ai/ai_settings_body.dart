// Settings > AI: the agent first (an add-on section placed 'top', with "Run
// AI on" and what that runner uses), then where Ask AI runs (the agent on
// the hub or on this computer, per "Run AI on"), the hub's agent tokens,
// and the recent tasks with their status. Phones only list tasks: computers
// answer them.

import 'package:flutter/widgets.dart';

import '../../src/rust/api/ai.dart' show AiRunner, AiRunnerKind;
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../../shell/addons.dart';
import 'agent_tokens_section.dart';
import 'ai_settings.dart';
import 'ai_store.dart';
import 'ask_ai.dart';
import 'connect_agent_section.dart';

class AiSettingsBody extends StatelessWidget {
  const AiSettingsBody({super.key});

  @override
  Widget build(BuildContext context) {
    final store = AiStore.of(context.app);
    final sections = appAddonContext.settingsSections.all
        .where(
          (s) => s.location == 'ai' && (s.isVisible?.call(context) ?? true),
        )
        .toList();
    final beforeTasks = sections.where((s) => s.placement == 'beforeTasks');
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final c = context.colors;
        final m = context.mobile;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SettingsHeading('AI', primary: true),
            Text(
              'Chat with the agent, or ask AI from the / menu on any device.',
              style: m
                  ? tsRaw(15, 21, 400, c.textSecondary)
                  : tsRaw(14, 20, 400, c.textSecondary),
            ),
            const SizedBox(height: 16),
            if (AiStore.canRun) ...[
              for (final section in sections.where(
                (s) => s.placement == 'top',
              ))
                section.build(context),
              const SettingsHeading('Ask AI', top: 24),
              // Ask AI always uses the agent, where "Run AI on" says: a hub
              // job, or this computer's agent.
              if (store.runsOn == AgentHost.hub)
                Text(
                  'Ask AI runs on the hub.',
                  key: const ValueKey('ai-runs-on-hub'),
                  style: m
                      ? tsRaw(15, 21, 400, c.textSecondary)
                      : tsRaw(14, 20, 400, c.textSecondary),
                )
              else
                Text(
                  'Ask AI runs on this computer.',
                  key: const ValueKey('ai-runs-on-this-computer'),
                  style: m
                      ? tsRaw(15, 21, 400, c.textSecondary)
                      : tsRaw(14, 20, 400, c.textSecondary),
                ),
            ],
            for (final section in beforeTasks) section.build(context),
            // Its `notes setup` commands run in a desktop terminal.
            if (!m)
              ConnectAgentSection(
                top: AiStore.canRun || beforeTasks.isNotEmpty ? 24 : 0,
              ),
            AgentTokensSection(
              top: !m || AiStore.canRun || beforeTasks.isNotEmpty ? 24 : 0,
            ),
            const SettingsHeading('Runner names', top: 24),
            _RunnerNames(store: store),
            const SettingsHeading('Recent tasks', top: 24),
            _TaskList(store: store),
          ],
        );
      },
    );
  }
}

class _TaskList extends StatelessWidget {
  const _TaskList({required this.store});
  final AiStore store;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    if (store.tasks.isEmpty) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 12),
        child: Text('No AI tasks yet.', key: const ValueKey('ai-no-tasks'), style: tsRaw(14, 20, 400, c.textTertiary)),
      );
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final t in store.tasks.take(20))
        Padding(
          key: ValueKey('ai-task-${t.id}'),
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            _StatusChip(t.status),
            const SizedBox(width: 10),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(t.prompt,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: m ? tsRaw(16, 22, 400, c.textPrimary) : tsRaw(14, 20, 400, c.textPrimary)),
                if (t.error.isNotEmpty)
                  Text(t.error,
                      maxLines: 3, overflow: TextOverflow.ellipsis, style: tsRaw(12, 16, 400, c.inlineCodeText)),
                Text(relativeTime(t.updatedMs.toInt()), style: tsRaw(12, 16, 400, c.textTertiary)),
              ]),
            ),
            if (t.status == AiStatus.failed) ...[
              const SizedBox(width: 8),
              OutlineButton(key: ValueKey('ai-retry-${t.id}'), label: 'Retry', onTap: () => store.retry(t.id)),
            ],
          ]),
        ),
    ]);
  }
}

class _StatusChip extends StatelessWidget {
  const _StatusChip(this.status);
  final String status;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final bg = switch (status) {
      AiStatus.done => c.okBg,
      AiStatus.failed => c.dangerBg,
      AiStatus.running => c.infoBg,
      _ => c.bgSecondary,
    };
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      decoration: BoxDecoration(color: bg, borderRadius: Radii.pill),
      child: Text(aiStatusLabel(status), style: tsRaw(12, 18, 600, c.textPrimary)),
    );
  }
}
