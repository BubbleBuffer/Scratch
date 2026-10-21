// A rendered note refreshes its persisted reference cache through the real
// bridge, without changing its ID anchors; notes search follows the summary.
import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/src/rust/api/agent.dart';

import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

OutlineChapter _chapter(
  AgentSessionState state,
  String title,
  String summary,
) => OutlineChapter(
  id: 'a-chapter',
  title: title,
  summary: summary,
  decisions: const [],
  ongoing: false,
  firstMessageId: state.messages.first.id,
  lastMessageId: state.messages.last.id,
  startedAt: state.messages.first.createdAt,
  endedAt: state.messages.last.createdAt,
  sections: const [],
);

void main() {
  setUpAll(initCore);
  appTest(
    'a rendered reference refreshes its stored titles and searchable summary by ID',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession());
      await real(tester, () => store.send('Say ok'));
      await settleUntil(
        tester,
        () =>
            !store.running &&
            (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
      );
      final agent = (await real(tester, store.ensureOpen))!;
      final state = store.state!;
      final initial = _chapter(state, 'First chapter', 'Cached summary.');
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: store.openId!,
          chapter: initial,
        ),
      );
      final ref = ChatReference.fromChapter(
        sessionId: store.openId!,
        chatTitle: 'First chapter',
        chapter: initial,
      );
      final page = await core(
        tester,
        (v) => v.createPage(title: 'Reference note'),
      );
      final block = newId();
      await core(
        tester,
        (v) => v.apply(page, [
          BOp(
            op: OpKind.insert,
            id: block,
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
      final ctx = tester.element(find.byType(Overlay).first);
      unawaited(openPage(ctx, page, 'Reference note'));
      await settle(tester);
      expect(find.text('First chapter › First chapter'), findsOneWidget);

      await real(
        tester,
        () => store.renameSession(store.openId!, 'User chat name'),
      );
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: store.openId!,
          chapter: _chapter(
            state,
            'Retitled chapter',
            'Fresh zebrapear summary.',
          ),
        ),
      );
      await settleUntil(
        tester,
        () => find
            .text('User chat name › Retitled chapter')
            .evaluate()
            .isNotEmpty,
      );
      await settle(tester);
      await real(tester, () => tv.app.pageStore.lastSave);
      final saved = await blockById(tester, page, block);
      final refreshed = ChatReference.fromTarget(saved.target)!;
      expect(refreshed.chatTitle, 'User chat name');
      expect(refreshed.chapterTitle, 'Retitled chapter');
      expect(refreshed.firstMessageId, ref.firstMessageId);
      expect(refreshed.lastMessageId, ref.lastMessageId);
      expect(saved.text, 'Fresh zebrapear summary.');
      final hits = await core(tester, (v) => v.search('zebrapear', limit: 10));
      expect(hits.map((h) => h.id), contains(page));
      expect(tester.takeException(), isNull);
    },
  );
}
