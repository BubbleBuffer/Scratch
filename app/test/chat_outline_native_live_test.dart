// A genuine fake-provider run writes messages through the normal harness;
// outline-only API writes then reach the mounted chat through revision polling.
// This is a plumbing check, not a replay or segmentation-engine demonstration.
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart';
import 'package:notes_app/src/rust/api/agent.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart';

OutlineChapter _chapter(AgentSessionState state, String title) =>
    OutlineChapter(
      id: 'topic-chapter',
      title: title,
      summary: 'A searchable topic summary.',
      decisions: const ['Keep the message anchors.'],
      ongoing: true,
      firstMessageId: state.messages.first.id,
      lastMessageId: state.messages.last.id,
      startedAt: state.messages.first.createdAt,
      endedAt: null,
      sections: const [],
    );

void main() {
  setUpAll(initCore);

  appTest(
    'native outline-only writes update the visible chat and automatic title, while a rename sticks',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession());
      await openAgentScreen(tester);
      await real(tester, () => store.send('Say ok'));
      await settleUntil(
        tester,
        () =>
            !store.running &&
            (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
      );
      final agent = (await real(tester, store.ensureOpen))!;
      final state = store.state!;
      final before = state.seq;
      final chapter = _chapter(state, 'Current topic');
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: store.openId!,
          chapter: chapter,
        ),
      );
      await settleUntil(
        tester,
        () =>
            store.outline.chapters.isNotEmpty &&
            store.state?.session.title == 'Current topic',
      );
      expect(
        store.state!.seq,
        before,
        reason: 'there was no message event to trigger the outline load',
      );
      expect(find.text('Current topic'), findsWidgets);

      final section = OutlineSection(
        id: 'a-section',
        title: 'Current section',
        summary: 'Section summary.',
        decisions: const [],
        firstMessageId: chapter.firstMessageId,
        lastMessageId: chapter.lastMessageId,
      );
      await real(
        tester,
        () => agent.upsertOutlineSection(
          sessionId: store.openId!,
          chapterFirstMessageId: chapter.firstMessageId,
          section: section,
        ),
      );
      await settleUntil(
        tester,
        () => store.outline.chapters.single.sections.isNotEmpty,
      );
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('outline-where')));
      expect(find.text('Current section'), findsWidgets);
      await tapAndSettle(
        tester,
        find.byKey(const ValueKey('outline-contents-section-a-section')),
      );

      await real(
        tester,
        () => store.renameSession(store.openId!, 'My chosen name'),
      );
      final retitled = _chapter(state, 'Later chapter title');
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: store.openId!,
          chapter: retitled,
        ),
      );
      await settleUntil(
        tester,
        () => store.outline.chapters.single.title == 'Later chapter title',
      );
      final loaded = await real(
        tester,
        () => agent.sessionState(sessionId: store.openId!),
      );
      expect(loaded.session.title, 'My chosen name');
      expect(store.state!.session.title, 'My chosen name');
      expect(
        store.outline.chapters.single.sections.single.title,
        'Current section',
      );
      expect(tester.takeException(), isNull);
    },
  );
}
