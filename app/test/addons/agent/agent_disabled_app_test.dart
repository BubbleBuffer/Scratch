// The whole app with the optional agent add-on switched off at boot. A chat
// reference already in a note draws as the unsupported-block placeholder, the
// note still opens, and the agent store refuses to open. Runs in its own file
// so the process-wide registrations start from this choice.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import '../../addon_harness.dart' show registerBuiltins;
import 'package:notes_app/services/addon_services.dart' show openPage;

import '../../chat_reference_fixture.dart';
import '../../harness.dart';

void main() {
  setUpAll(() async {
    registerBuiltins(disabledAddons: {'agent'});
    await initCore();
  });

  appTest(
    'with agent disabled, a chat reference draws as a placeholder and its note opens',
    (tester) async {
      expect(blockRegistry.has(Kinds.chatRef), isFalse);
      expect(noteTypeRegistry.of('chat'), isNull);
      expect(activeAddonContext!.installedIds, isNot(contains('agent')));

      await pumpApp(tester, mobile: false, empty: true);
      final ref = ChatReference.fromChapter(
        sessionId: 'sess-1',
        chatTitle: chatRefChatTitle,
        chapter: chatRefChapter(),
      );
      final page = await core(
        tester,
        (v) => v.createPage(title: 'Disabled chat note'),
      );
      final block = newId();
      await core(
        tester,
        (v) => v.apply(page, [
          BOp(
            op: OpKind.insert,
            id: block,
            after: null,
            kind: Kinds.chatRef,
            text: ref.searchText,
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.setTarget,
            id: block,
            kind: Kinds.chatRef,
            text: '',
            marks: const [],
            checked: false,
            target: ref.toTarget(),
          ),
        ]),
      );

      unawaited(
        openPage(
          tester.element(find.byType(Overlay).first),
          page,
          'Disabled chat note',
        ),
      );
      await settle(tester);

      expect(find.text('chat_ref · ${ref.searchText}'), findsOneWidget);
      expect(find.byType(ChatReferenceCard), findsNothing);
      expect(find.text('Disabled chat note'), findsWidgets);
    },
  );

  appTest('the agent store cannot open while its add-on is disabled', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final store = AgentStore.of(tv.app);
    expect(store.enabled, isFalse);
    expect(await tester.runAsync(() => store.ensureOpen()), isNull);
    expect(store.openId, isNull);
  });
}
