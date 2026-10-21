// Chat references: the factories map the outline, labels follow the rule
// (Chat › Chapter, plus the section only when it is the target), and the
// JSON payload round-trips and is rejected when damaged.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/agent.dart';

import 'chat_reference_fixture.dart';

OutlineChapter _chapter() => chatRefChapter();

void main() {
  test('fromChapter takes the ids, cached names, decision and dates of the chapter', () {
    final ref = ChatReference.fromChapter(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: _chapter(),
    );

    expect(ref.level, ChatRefLevel.chapter);
    expect(ref.sessionId, 'sess-1');
    expect(ref.firstMessageId, 'm-40');
    expect(ref.lastMessageId, 'm-88');
    expect(ref.title, 'Native drag-and-drop');
    expect(ref.crumb, 'BubblePlay cursor › Native drag-and-drop');
    expect(
      ref.label,
      ref.crumb,
      reason: 'a chapter target has no section part',
    );
    expect(
      ref.decision,
      'Ship for Chrome first; Explorer stays behind a flag.',
    );
    expect(ref.when, 'Sep 2 – Sep 4');
    expect(ref.sections.map((s) => s.title), [
      'Web to PC path and the handoff stall',
      'Chrome live drops pass, Explorer still blocked',
    ]);
    expect(ref.sections.last.firstMessageId, 'm-53');
    expect(ref.sections.last.decision, 'Keep Explorer behind a flag.');
    expect(
      ref.sections.first.decision,
      isNull,
      reason: 'an empty decision list has no decision',
    );
  });

  test('an ongoing chapter reads as since its start, with no end', () {
    final chapter = OutlineChapter(
      id: 'ch-3',
      title: 'Sync retries',
      summary: 'Retry policy in progress.',
      decisions: const [],
      ongoing: true,
      startedAt: '2026-09-05T09:00:00Z',
      endedAt: null,
      firstMessageId: 'm-90',
      lastMessageId: 'm-95',
      sections: const [],
    );
    final ref = ChatReference.fromChapter(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: chapter,
    );

    expect(ref.when, 'since Sep 5');
    expect(ref.decision, isNull);
    expect(ref.sections, isEmpty);
  });

  test('fromSection targets one section: its range, and the label adds it', () {
    final chapter = _chapter();
    final ref = ChatReference.fromSection(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: chapter,
      section: chapter.sections.last,
    );

    expect(ref.level, ChatRefLevel.section);
    expect(ref.firstMessageId, 'm-53');
    expect(ref.lastMessageId, 'm-88');
    expect(ref.title, 'Chrome live drops pass, Explorer still blocked');
    expect(ref.crumb, 'BubblePlay cursor › Native drag-and-drop');
    expect(
      ref.label,
      'BubblePlay cursor › Native drag-and-drop › Chrome live drops pass, Explorer still blocked',
    );
    expect(ref.decision, 'Keep Explorer behind a flag.');
    expect(ref.when, isEmpty, reason: 'a section has no dates of its own');
    expect(ref.sections, isEmpty, reason: 'sections are single');
  });

  test('the block text for search is the summary', () {
    final ref = ChatReference.fromChapter(
      sessionId: 's',
      chatTitle: 'C',
      chapter: _chapter(),
    );
    expect(
      ref.searchText,
      'Virtual-file drag from the web to the PC: built, deployed.',
    );
  });

  test('the payload round-trips through the block target', () {
    final ref = ChatReference.fromChapter(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: _chapter(),
    );
    final back = ChatReference.fromTarget(ref.toTarget());

    expect(back, isNotNull);
    expect(back!.toJson(), ref.toJson());
    expect(back.label, ref.label);
    expect(back.when, ref.when);
    expect(back.sections.length, 2);
  });

  test('a section payload round-trips with its section title', () {
    final chapter = _chapter();
    final ref = ChatReference.fromSection(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: chapter,
      section: chapter.sections.first,
    );
    final back = ChatReference.fromTarget(ref.toTarget())!;
    expect(back.isSection, isTrue);
    expect(back.sectionTitle, 'Web to PC path and the handoff stall');
    expect(back.label, ref.label);
  });

  test('a missing, damaged or foreign target reads as no reference', () {
    expect(ChatReference.fromTarget(null), isNull);
    expect(ChatReference.fromTarget(''), isNull);
    expect(ChatReference.fromTarget('not json'), isNull);
    expect(
      ChatReference.fromTarget('{"level":"chapter"}'),
      isNull,
      reason: 'missing ids',
    );
    expect(
      ChatReference.fromTarget('{"level":"galaxy","sessionId":"s"}'),
      isNull,
      reason: 'unknown level',
    );
  });

  test('band colours are stable for one id', () {
    expect(chatRefBandFor('ch-2'), chatRefBandFor('ch-2'));
    expect(chatRefBands, contains(chatRefBandFor('ch-2')));
  });
}
