// Golden screenshots of the phone Agent chat (the full-page route that
// `openAgent` pushes), light and dark, on the real core and agent bridge
// (BubblePaw's fake extension answers "ok"): an empty chat and a conversation
// with a work line and an approval card. They pin the phone layout of the chat
// pane against the layout it replaced.
// Regenerate with: flutter test --update-goldens test/mobile_agent_chat_golden_test.dart

@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/home.dart';

import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> shot(String name) async {
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('goldens/$name.png'),
  );
}

Future<void> _open(WidgetTester tester) async {
  unawaited(openAgent(tester.element(find.byType(HomeScreen))));
  await settle(tester);
  expect(key('agent-screen'), findsOneWidget);
}

void main() {
  setUpAll(initCore);

  for (final dark in [false, true]) {
    final tag = 'phone-${dark ? 'dark' : 'light'}';

    appTest('mobile agent chat empty $tag', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark);
      await openAgentStore(tester);
      await _open(tester);
      await shot('mobile-agent-chat-empty-$tag');
    });

    appTest('mobile agent chat conversation $tag', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark);
      final store = await openAgentStore(tester);
      await _open(tester);
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
          '["Garden plan", "Seed list"].length',
        ),
      );
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
      await shot('mobile-agent-chat-conversation-$tag');
    });
  }
}
