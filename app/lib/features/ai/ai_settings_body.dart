// Settings > Agent: the page title and intro, the agent add-on's sections
// (location 'ai': on a computer "Use the agent", "Where it runs", "What it
// may do", "On this computer" and Advanced; on a phone the hub-only
// version), then the recent Ask AI tasks from every device with their
// status and Retry. MCP servers, quick setup and tokens are on Settings >
// Connectors.

import 'package:flutter/widgets.dart';

import '../../shell/addons.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/settings_kit.dart';
import 'ask_ai.dart';
import 'ai_store.dart';

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
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) => SPage(
        title: 'Agent',
        intro:
            'The agent is Scratch’s built-in AI. It answers in the Agent chat and when you choose **Ask AI** from the / menu.',
        children: [
          for (final s in sections.where((s) => s.placement == 'top'))
            s.build(context),
          for (final s in sections.where((s) => s.placement == 'beforeTasks'))
            s.build(context),
          SSection(
            title: 'Recent tasks',
            intro: 'Ask AI requests from all your devices.',
            children: [
              const SizedBox(height: 12),
              _TaskList(store: store),
            ],
          ),
        ],
      ),
    );
  }
}

/// The command that adds an MCP endpoint to Claude Code.
String claudeAddCommand(String url, String token) =>
    'claude mcp add --transport http notes $url --header "Authorization: Bearer $token"';

class _TaskList extends StatelessWidget {
  const _TaskList({required this.store});
  final AiStore store;

  @override
  Widget build(BuildContext context) {
    if (store.tasks.isEmpty) {
      return const SEmpty(
        key: ValueKey('ai-no-tasks'),
        icon: Sx.sparkles,
        title: 'No AI tasks yet',
        text: 'Select text, or type / on any page and choose Ask AI.',
      );
    }
    return SList(
      children: [
        for (final t in store.tasks.take(20))
          KeyedSubtree(
            key: ValueKey('ai-task-${t.id}'),
            child: SItem(
              icon: Sx.sparkles,
              name: t.prompt,
              sub: relativeTime(t.updatedMs.toInt()),
              error: t.error.isEmpty ? null : t.error,
              status: SStatus(
                aiStatusLabel(t.status),
                kind: switch (t.status) {
                  AiStatus.done => SStatusKind.ok,
                  AiStatus.failed => SStatusKind.bad,
                  AiStatus.running => SStatusKind.info,
                  _ => SStatusKind.off,
                },
              ),
              trailing: [
                if (t.status == AiStatus.failed)
                  SButton(
                    'Retry',
                    key: ValueKey('ai-retry-${t.id}'),
                    kind: SButtonKind.quiet,
                    onTap: () => store.retry(t.id),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}
