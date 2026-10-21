// Opt-in acceptance check against an actual import-replay result. Keep the
// replay and its raw messages outside the repository and normal fixtures.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'agent_test.dart' show fakeAgentSettings, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);
  final source = Platform.environment['NOTES_OUTLINE_REPLAY_DATA_DIR'];
  appTest(
    'the imported replay opens with three chapters and eight anchored sections',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final destination = Directory('${current.dir.path}/agent')
        ..createSync(recursive: true);
      for (final name in ['agent.sqlite', 'chat_outline.sqlite']) {
        File('$source/agent/$name').copySync('${destination.path}/$name');
      }
      final store = AgentStore.attach(
        current.vault,
        fakeAgentSettings(),
        dataDir: current.dir.path,
      );
      await real(tester, store.ensureOpen);
      expect(store.openError, isNull);
      expect(store.sessions, hasLength(1));
      await real(tester, () => store.openSession(store.sessions.single.id));
      expect(store.openError, isNull);
      final state = store.state!;
      final chapters = store.outline.chapters;
      expect(chapters, hasLength(3));
      expect(chapters.expand((c) => c.sections), hasLength(8));
      final order = {
        for (var i = 0; i < state.messages.length; i++) state.messages[i].id: i,
      };
      for (final chapter in chapters) {
        expect(order, contains(chapter.firstMessageId));
        expect(order, contains(chapter.lastMessageId));
        final start = order[chapter.firstMessageId]!;
        final end = order[chapter.lastMessageId]!;
        expect(start, lessThanOrEqualTo(end));
        for (final section in chapter.sections) {
          expect(order, contains(section.firstMessageId));
          expect(order, contains(section.lastMessageId));
          expect(order[section.firstMessageId]!, greaterThanOrEqualTo(start));
          expect(order[section.lastMessageId]!, lessThanOrEqualTo(end));
          expect(
            order[section.firstMessageId]!,
            lessThanOrEqualTo(order[section.lastMessageId]!),
          );
        }
      }
      await openAgentScreen(tester);
      expect(key('agent-screen'), findsOneWidget);
      await tapAndSettle(tester, key('outline-where'));
      await tapAndSettle(
        tester,
        key('outline-contents-chapter-${chapters.first.id}'),
      );
      expect(key('outline-chapter-${chapters.first.id}'), findsOneWidget);
      expect(find.text('Hide messages'), findsOneWidget);
      final section = chapters.last.sections.last;
      await tapAndSettle(tester, key('outline-where'));
      final entry = key('outline-contents-section-${section.id}');
      await tester.ensureVisible(entry);
      await tapAndSettle(tester, entry);
      expect(key('outline-section-sec:${section.id}'), findsOneWidget);
      expect(
        find.descendant(
          of: key('outline-where'),
          matching: find.textContaining(section.title),
        ),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
    skip: source == null,
  );
}
