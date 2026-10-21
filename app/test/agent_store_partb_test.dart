import 'package:flutter_test/flutter_test.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'edit Undo uses the store and refusal preserves a later user change',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Undo edits'),
      ))!;
      final page = tv.pages['Garden plan']!;
      final before = await real(tester, () => tv.vault.loadPage(page));
      await real(
        tester,
        () => store.runScript(
          session.id,
          'Edit the garden',
          'await tools.edit_page({page:"Garden plan",op:"set_body",markdown:"Plant basil."})',
        ),
      );
      await settle(tester);
      final edit = store.state!.edits.single;
      await tapAndSettle(tester, key('agent-edit-${edit.id}-undo'));
      await settleUntil(tester, () => store.state!.edits.single.undone);
      expect(find.text('Undone'), findsOneWidget);
      final restored = await real(tester, () => tv.vault.loadPage(page));
      expect(
        restored.blocks.map((b) => b.text).toList(),
        before.blocks.map((b) => b.text).toList(),
      );

      final run = (await real(
        tester,
        () => store.runScript(
          session.id,
          'Another edit',
          'await tools.edit_page({page:"Garden plan",op:"set_body",markdown:"Plant parsley."})',
        ),
      ))!;
      await settle(tester);
      final second = store.state!.edits.firstWhere((e) => e.runId == run.id);
      await real(
        tester,
        () => tv.vault.renamePage(page, 'My newer garden title'),
      );
      await tester.ensureVisible(key('agent-edit-${second.id}-undo'));
      await tapAndSettle(tester, key('agent-edit-${second.id}-undo'));
      expect(
        textOf(tester, 'agent-edit-${second.id}-refusal'),
        contains('page changed since the agent\'s edit; use page history'),
      );
      expect(
        (await real(tester, () => tv.vault.loadPage(page))).title,
        'My newer garden title',
      );
    },
  );

  appTest(
    'visible chat subscribes to real page actions and closing unregisters it',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Page actions'),
      ))!;
      await settle(tester);
      await real(
        tester,
        () => store.runScript(
          session.id,
          'Show the recipes',
          'await tools.ui_open_page({page:"Recipes"})',
        ),
      );
      await settle(tester);
      expect(tv.app.ui.lastPage, tv.pages['Recipes']);
      expect(key('agent-panel'), findsOneWidget);
      expect(
        store.state!.messages.where((m) => m.role == 'tool').last.content,
        contains('shown to the user'),
      );
      await tapAndSettle(tester, key('agent-panel-close'));
      await settle(tester);
      await real(
        tester,
        () => store.runScript(
          session.id,
          'No visible chat',
          'await tools.ui_open_page({page:"Budget"})',
        ),
      );
      await settle(tester);
      expect(tv.app.ui.lastPage, tv.pages['Recipes']);
      expect(
        store.state!.messages.where((m) => m.role == 'tool').last.content,
        contains('no app is showing this chat; nothing opened'),
      );
    },
  );
}
