// Golden screenshots of the Agent section at 1440x900, light and dark, on
// the real core and agent bridge (BubblePaw's fake extension answers "ok"):
// - empty: no chats yet;
// - a conversation: a question and its answer, a compact work line, and an
//   approval card (fake state); code stays hidden by default;
// - markdown: a reply with headings, nested lists, a quote, a divider, links
//   and a code fence (the reply is fake state: the fake answers "ok"), under
//   a user message whose markdown stays plain.
// Regenerate with: flutter test --update-goldens test/agent_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'agent_markdown_test.dart' show chatWithReply;
import 'agent_test.dart' show openAgentScreen, openAgentStore;
import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> shot(String name) async {
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('../screenshots/$name.png'),
  );
}

void main() {
  setUpAll(initCore);

  for (final dark in [false, true]) {
    final tag = 'desktop-${dark ? 'dark' : 'light'}';

    appTest('agent empty $tag', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark);
      await openAgentStore(tester);
      await openAgentScreen(tester);
      await shot('$tag-agent-empty');
    });

    appTest('settings agent $tag', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark);
      final store = await openAgentStore(tester);
      final ai = await aiStore(tester, settings: store.settings);
      await settleUntil(tester, () => store.hostInfo != null);
      store.useAppSettings(current.app.settings);
      // Model cache contents vary by machine; keep this settings screenshot
      // deterministic while retaining the real bridge beneath the fixture.
      store.models = const [
        AgentModel(id: 'claude-code/fake', name: 'Fake'),
        AgentModel(id: 'claude-code/claude-fable-5-1', name: 'Fable 5.1'),
        AgentModel(id: 'claude-code/claude-opus-5-5', name: 'Opus 5.5'),
        AgentModel(id: 'claude-code/claude-sonnet-5-5', name: 'Sonnet 5.5'),
        AgentModel(id: 'claude-code/claude-haiku-5-5', name: 'Haiku 5.5'),
      ];
      // Fixed display values after opening keep machine and fixture paths
      // out of the screenshot; the real bridge remains open underneath.
      store.settings
        ..agentBunPath = 'bun'
        ..agentClaudeCodeDir = r'C:\Tools\BubblePaw\extensions\claude-code'
        ..desktopRoot = r'C:\Users\Example';
      store.projects = const [
        AgentProject(
          id: 'home',
          name: 'Home',
          path: r'C:\Users\Example',
          addedAt: '2026-10-09T12:00:00Z',
        ),
      ];
      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-ai'));
      await Scrollable.ensureVisible(
        tester.element(key('agent-enabled-toggle')),
        alignment: 0.13,
      );
      await settle(tester);
      expect(key('agent-host-info').hitTestable(), findsOneWidget);
      expect(key('agent-projects'), findsOneWidget);
      await shot('$tag-settings-agent');
      await closeAi(tester, ai);
    });

    appTest('agent conversation $tag', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final s = (await real(
        tester,
        () => store.createSession(title: 'Garden questions'),
      ))!;
      await real(tester, () => store.send('Which pages mention the garden?'));
      await settleUntil(
        tester,
        () =>
            (store.state?.messages.any(
                  (m) => m.role == 'assistant' && m.content == 'ok',
                ) ??
                false) &&
            !store.running,
        timeout: const Duration(seconds: 30),
      );
      await real(
        tester,
        () => store.runScript(
          s.id,
          'Find the garden pages',
          '// Titles that start with "Garden"\n'
              '["Garden plan", "Seed list", "Garden tools"].filter((t) => t.startsWith("Garden"))',
        ),
      );
      await settle(tester);
      // Code is available through the work details and stays hidden by default.
      store.debugAddApproval(
        AgentApproval(
          id: 'approval-1',
          sessionId: s.id,
          runId: 'run',
          callId: 'call',
          tool: 'delete_page',
          argumentsJson: '{"page":"Seed list","permanent":false}',
          state: 'pending',
          createdAt: '2026-10-06T11:20:00Z',
        ),
      );
      await settle(tester);
      await shot('$tag-agent-conversation');
    });

    appTest('agent markdown $tag', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark);
      await chatWithReply(
        tester,
        goldenMarkdown,
        question: 'Plan the **garden** for spring?',
      );
      await shot('$tag-agent-markdown');
    });
  }
}

/// The markdown golden's reply.
const goldenMarkdown = '''## Spring plan

The **seed order** arrives *Friday*. See [[Garden plan]] for the beds and [the planting guide](https://example.com/guide); run `water()` each morning.

### Beds

- North bed
  - Tomatoes
  - Basil
- South bed

1. Dig the beds
2. Add compost
3. Plant

> Water before 9 am in hot weeks.

---

```js
await tools.create_page({title: "Spring sowing"})
```''';
