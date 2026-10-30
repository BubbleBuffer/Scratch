// The composer's input handling, pinned before the kit's composer takes it
// over: answers to older `@` queries never replace newer ones, and a send
// that does not happen keeps the draft, the picked mentions and the quoted
// selection.
import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/mention_menu.dart' show MentionItem;
import 'package:notes_app/state/editor_focus.dart';

import 'agent_chat_partb_test.dart' show showWidget;
import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

String _draft(WidgetTester tester) =>
    tester.widget<EditableText>(key('agent-composer')).controller.text;

void main() {
  setUpAll(initCore);

  appTest(
    'an older @ answer that arrives late does not replace the newer menu',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Late answers'));
      final answers = <String, Completer<List<MentionItem>>>{};
      final asked = <String>[];
      await showWidget(
        tester,
        AgentComposer(
          store: store,
          mentionSource: (context, query, exclude) {
            asked.add(query);
            return (answers[query] = Completer<List<MentionItem>>()).future;
          },
        ),
      );
      await tester.tap(key('agent-composer'));
      await tester.enterText(key('agent-composer'), '@G');
      await tester.pump();
      await tester.enterText(key('agent-composer'), '@Ga');
      await tester.pump();
      expect(asked, ['G', 'Ga']);

      // The newer query is answered first and shows its page.
      answers['Ga']!.complete([
        const MentionItem.page('garden', 'Garden', null),
      ]);
      await settle(tester);
      expect(key('mention-page-garden'), findsOneWidget);

      // The older answer then arrives and is ignored.
      answers['G']!.complete([
        const MentionItem.page('groceries', 'Groceries', null),
      ]);
      await settle(tester);
      expect(key('mention-page-groceries'), findsNothing);
      expect(key('mention-page-garden'), findsOneWidget);
    },
  );

  appTest('an older @ answer is dropped while the newer one is pending', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Pending answers'));
    final answers = <String, Completer<List<MentionItem>>>{};
    await showWidget(
      tester,
      AgentComposer(
        store: store,
        mentionSource: (context, query, exclude) =>
            (answers[query] = Completer<List<MentionItem>>()).future,
      ),
    );
    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), '@G');
    await tester.pump();
    await tester.enterText(key('agent-composer'), '@Ga');
    await tester.pump();

    answers['G']!.complete([
      const MentionItem.page('groceries', 'Groceries', null),
    ]);
    await settle(tester);
    expect(key('agent-mentions'), findsNothing);

    answers['Ga']!.complete([
      const MentionItem.page('garden', 'Garden', null),
    ]);
    await settle(tester);
    expect(key('mention-page-garden'), findsOneWidget);
    expect(key('mention-page-groceries'), findsNothing);
  });

  appTest('an answer after the @ is gone shows no menu', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Gone mention'));
    final answer = Completer<List<MentionItem>>();
    await showWidget(
      tester,
      AgentComposer(
        store: store,
        mentionSource: (context, query, exclude) => answer.future,
      ),
    );
    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), '@G');
    await tester.pump();
    await tester.enterText(key('agent-composer'), 'Hello');
    await tester.pump();
    answer.complete([const MentionItem.page('garden', 'Garden', null)]);
    await settle(tester);
    expect(key('agent-mentions'), findsNothing);
    expect(_draft(tester), 'Hello');
  });

  appTest(
    'a send that fails keeps the draft, the mention and the quoted selection',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Failing send'));
      final garden = tv.pages['Garden plan']!;
      final recipes = tv.pages['Recipes']!;
      await core(
        tester,
        (vault) => vault.apply(garden, [
          BOp(
            op: OpKind.insert,
            id: newId(),
            kind: Kinds.paragraph,
            text: 'Passage the user quoted.',
            marks: const [],
            checked: false,
          ),
        ]),
      );
      final page = await real(tester, () => tv.vault.loadPage(garden));
      final block = page.blocks.firstWhere((b) => b.text.isNotEmpty);
      EditorFocus.forApp(tv.app).update(
        pageId: garden,
        pageTitle: 'Garden plan',
        blockIds: [block.id],
        text: block.text,
      );
      await showWidget(
        tester,
        AgentComposer(
          store: store,
          panel: true,
          mentionSource: (context, query, exclude) async => [
            MentionItem.page(recipes, 'Recipes', null),
          ],
        ),
      );
      await tester.tap(key('agent-composer'));
      await tester.enterText(key('agent-composer'), '@Rec');
      await settle(tester);
      await tapAndSettle(tester, key('mention-page-$recipes'));
      expect(key('agent-context-mention-$recipes'), findsOneWidget);
      expect(key('agent-context-selection'), findsOneWidget);
      expect(key('agent-context-page'), findsOneWidget);

      // A model no provider serves makes the send itself fail.
      store.pickModel(store.openId, 'missing-provider/model');
      await tapAndSettle(tester, key('agent-send'));
      await settleUntil(tester, () => store.error != null);

      expect(store.error, isNotNull);
      expect(_draft(tester), '@Recipes ');
      expect(key('agent-context-mention-$recipes'), findsOneWidget);
      expect(key('agent-context-selection'), findsOneWidget);
      expect(key('agent-context-page'), findsOneWidget);
      expect(store.state?.messages.where((m) => m.role == 'user'), isEmpty);

      // With a working model the same draft goes out and the composer clears.
      store.pickModel(store.openId, 'claude-code/code-model');
      await tapAndSettle(tester, key('agent-send'));
      await settleUntil(
        tester,
        () => store.state?.messages.any((m) => m.role == 'user') ?? false,
      );
      expect(store.error, isNull);
      expect(_draft(tester), isEmpty);
      expect(key('agent-context-mention-$recipes'), findsNothing);
      // The panel takes the page's current passage again for the next message.
      expect(key('agent-context-selection'), findsOneWidget);
      final message = store.state!.messages.firstWhere((m) => m.role == 'user');
      expect(message.text, '@Recipes');
      expect(
        message.context!.pages.map((p) => p.id),
        containsAll([recipes, garden]),
      );
      expect(message.context!.selection!.blockIds, [block.id]);
    },
  );

  appTest('a chat that cannot be created keeps the draft', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    expect(store.openId, isNull);
    await showWidget(tester, AgentComposer(store: store));
    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), 'First line\nSecond line');
    await settle(tester);

    // The agent is turned off before the first send: no chat can be made.
    store.settings.agentEnabled = false;
    await tapAndSettle(tester, key('agent-send'));
    await settle(tester);

    expect(store.sessions, isEmpty);
    expect(_draft(tester), 'First line\nSecond line');
  });

  appTest('a send while the agent is off fails visibly and keeps the draft', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Agent off'));
    await showWidget(tester, AgentComposer(store: store));
    await tester.tap(key('agent-composer'));
    await tester.enterText(key('agent-composer'), 'Do not lose me');
    await settle(tester);

    store.settings.agentEnabled = false;
    await tapAndSettle(tester, key('agent-send'));
    await settle(tester);

    expect(store.error, isNotNull);
    expect(_draft(tester), 'Do not lose me');
    expect(store.state?.messages.where((m) => m.role == 'user'), isEmpty);
  });
}
