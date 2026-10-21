import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/state/editor_focus.dart';

import 'agent_chat_partb_test.dart' show showWidget;
import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  for (final mentioned in [false, true]) {
    appTest(
      'untitled ${mentioned ? 'mentioned' : 'automatic'} composer page shows its first line',
      (tester) async {
        await pumpApp(tester, mobile: false);
        final store = await openAgentStore(tester);
        await real(tester, () => store.createSession(title: 'Page context'));
        final pageId = await core(tester, (v) async {
          final id = await v.createPage();
          await v.apply(id, [
            BOp(
              op: OpKind.insert,
              id: newId(),
              kind: Kinds.paragraph,
              text: 'Groceries for Sunday',
              marks: const [],
              checked: false,
            ),
          ]);
          return id;
        });
        if (!mentioned) {
          EditorFocus.forApp(current.app).update(pageId: pageId, pageTitle: '');
        }
        await showWidget(
          tester,
          AgentComposer(store: store, panel: !mentioned),
        );
        if (mentioned) {
          await tester.enterText(key('agent-composer'), '@Groceries');
          await settleUntil(
            tester,
            () => key('mention-page-$pageId').evaluate().isNotEmpty,
          );
          await tapAndSettle(tester, key('mention-page-$pageId'));
        }
        final chip = key(
          mentioned ? 'agent-context-mention-$pageId' : 'agent-context-page',
        );
        final name = find.descendant(
          of: chip,
          matching: find.text('Groceries for Sunday'),
        );
        await settleUntil(tester, () => name.evaluate().isNotEmpty);
        expect(name, findsOneWidget);
        expect(
          find.descendant(of: chip, matching: find.text('Untitled')),
          findsNothing,
        );
      },
    );
  }
}
