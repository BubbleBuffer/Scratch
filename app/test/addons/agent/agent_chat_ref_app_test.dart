// A chat reference in a note, drawn by the whole app with the agent add-on
// enabled. The chat note type is substituted in the app's registry; the
// block must draw through it with the reference's session and range target,
// not through the agent's own embed.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/services/addon_services.dart' show openPage;

import '../../chat_reference_fixture.dart';
import '../../harness.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'a chat reference draws through the substituted chat type with its session and range',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final chapter = chatRefChapter();
      final ref = ChatReference.fromChapter(
        sessionId: 'sess-1',
        chatTitle: chatRefChatTitle,
        chapter: chapter,
      );
      final page = await core(
        tester,
        (v) => v.createPage(title: 'Substituted chat note'),
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

      final received = <NoteArgs>[];
      Widget substitute(BuildContext context, NoteArgs args) {
        received.add(args);
        return const Text('Substituted chat embed');
      }

      final agentChat = noteTypeRegistry.of('chat')!;
      addTearDown(() => noteTypeRegistry.register(agentChat));
      noteTypeRegistry.register(
        NoteType(
          id: 'chat',
          label: 'Chat stand-in',
          chip: substitute,
          card: substitute,
          embed: substitute,
          fullPage: substitute,
          bodySource: 'agent',
          usesEditor: false,
        ),
      );

      unawaited(
        openPage(
          tester.element(find.byType(Overlay).first),
          page,
          'Substituted chat note',
        ),
      );
      await settle(tester);

      expect(find.text('Substituted chat embed'), findsWidgets);
      expect(find.byType(ChatReferenceCard), findsNothing);
      expect(received, isNotEmpty);
      final args = received.last;
      expect(args.pageId, 'sess-1');
      expect(args.pageType, 'chat');
      expect(args.target, ref.toTarget());
      expect(args.instanceId, block);

      final range = ChatReference.fromTarget(args.target)!;
      expect(range.sessionId, 'sess-1');
      expect(range.firstMessageId, chapter.firstMessageId);
      expect(range.lastMessageId, chapter.lastMessageId);
    },
  );
}
