@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/state/editor_focus.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'page_stack_golden_test.dart' show openSheet, topEditor;
import 'sync_helpers.dart';

Future<void> shot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../screenshots/$name.png'),
);

void main() {
  setUpAll(initCore);
  for (final dark in [false, true]) {
    final tag = 'desktop-${dark ? 'dark' : 'light'}';
    appTest('agent page panel $tag', (tester) async {
      final tv = await pumpApp(tester, mobile: false, dark: dark);
      final store = await openAgentStore(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Spring garden'),
      ))!;
      await real(
        tester,
        () => store.runScript(
          session.id,
          'Plan the spring garden.',
          'await tools.search_notes({query:"garden"}); '
              'await tools.read_page({page:"Garden plan"}); '
              'await tools.edit_page({page:"Garden plan",op:"set_body",markdown:'
              '"Keep the north bed for tomatoes.\\n\\nSow basil beside them.\\n\\nWater before 9 am."}); '
              'await tools.create_page({title:"Seed list",markdown:"- Tomatoes\\n- Basil"});',
        ),
      );
      await settle(tester);
      await openSheet(tester, tv.pages['Garden plan']!, 'Garden plan');
      await real(tester, () => toggleAgentPanel(topEditor(tester).context));
      await settle(tester);
      store.debugAddReply(
        'The garden plan is ready. I added the seed list and kept the watering plan together.',
      );
      await settle(tester);
      final edit = store.state!.edits.firstWhere((e) => e.kind == 'edited');
      await tapAndSettle(tester, find.text('Edited Garden plan'));
      expect(find.text('Created Seed list'), findsOneWidget);
      expect(find.text('Edited Garden plan'), findsOneWidget);
      await shot('$tag-agent-panel');
    });

    appTest('agent context chat $tag', (tester) async {
      final tv = await pumpApp(tester, mobile: false, dark: dark);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      await real(tester, () => store.createSession(title: 'Garden questions'));
      await real(
        tester,
        () => store.send(
          'What should I plant beside the tomatoes?',
          context: AgentMessageContext(
            pages: [
              AgentContextPage(
                id: tv.pages['Garden plan']!,
                title: 'Garden plan',
              ),
              AgentContextPage(id: tv.pages['Recipes']!, title: 'Recipes'),
            ],
            selection: AgentContextSelection(
              pageId: tv.pages['Garden plan']!,
              pageTitle: 'Garden plan',
              blockIds: const [],
              text: 'Keep the north bed for tomatoes.',
            ),
          ),
        ),
      );
      await settleUntil(
        tester,
        () =>
            !store.running &&
            (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
      );
      store.debugAddReply(
        'Basil is a good companion. Keep it close to the tomatoes so both are easy to water.',
      );
      await settle(tester);
      expect(find.text('Selection'), findsOneWidget);
      await shot('$tag-agent-context');
      // The full screen only attaches pages explicitly mentioned in its composer.
      expect(EditorFocus.forApp(tv.app).pageId, isNull);
    });
  }
}
