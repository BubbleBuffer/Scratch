// The chat outline mapped to the kit's: the layout and the widgets are the
// kit's (and tested there); this covers what Scratch feeds them. Dates are
// parsed, the range text follows from them, and a chapter's message count
// leaves tool results out. Pure Dart, no widgets.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_outline.dart';
import 'package:notes_app/src/rust/api/agent.dart';

AgentMessage _message(String id, String role) => AgentMessage(
  id: id,
  role: role,
  content: id,
  text: id,
  toolCalls: const [],
  isError: false,
  createdAt: '2026-10-09T12:00:00Z',
);

OutlineChapter _chapter({
  String id = 'A',
  String title = 'Chapter A',
  String summary = '',
  List<String> decisions = const [],
  bool ongoing = false,
  String first = 'u1',
  String last = 'a1',
  String startedAt = '2026-09-28T10:00:00Z',
  String? endedAt = '2026-09-30T10:00:00Z',
  List<OutlineSection> sections = const [],
}) => OutlineChapter(
  id: id,
  title: title,
  summary: summary,
  decisions: decisions,
  ongoing: ongoing,
  firstMessageId: first,
  lastMessageId: last,
  startedAt: startedAt,
  endedAt: endedAt,
  sections: sections,
);

void main() {
  test('no chapters: the kit gets the empty outline', () {
    expect(kitOutline(ChatOutline(chapters: const [])).isEmpty, isTrue);
  });

  test('a chapter keeps its ids, text and sections', () {
    final outline = kitOutline(
      ChatOutline(
        chapters: [
          _chapter(
            summary: 'Picked the spring plan. Then we costed it.',
            decisions: const ['Keep A'],
            sections: const [
              OutlineSection(
                id: 'S1',
                title: 'Chapter A: Start',
                summary: '',
                decisions: [],
                firstMessageId: 'u1',
                lastMessageId: 'a1',
              ),
            ],
          ),
        ],
      ),
    );
    final c = outline.chapters.single;
    expect((c.id, c.first, c.last), ('A', 'u1', 'a1'));
    expect(c.decisions, ['Keep A']);
    expect(c.gist, 'Picked the spring plan.');
    expect(c.sections.single.id, 'S1');
    // The section repeats its chapter's name; the kit strips it.
    expect(c.sections.single.title, 'Start');
  });

  test('dates: a closed chapter shows its range, an open one "since"', () {
    final closed = kitOutline(ChatOutline(chapters: [_chapter()]));
    expect(closed.chapters.single.when, isNot(contains('since')));
    expect(closed.chapters.single.when, contains('Sep'));
    final open = kitOutline(
      ChatOutline(chapters: [_chapter(ongoing: true, endedAt: null)]),
    );
    expect(open.chapters.single.when, startsWith('since Sep'));
    expect(open.chapters.single.ongoing, isTrue);
  });

  test('a start that is not a date has no range', () {
    final outline = kitOutline(
      ChatOutline(chapters: [_chapter(startedAt: 'soon')]),
    );
    expect(outline.chapters.single.when, '');
  });

  test('a closed chapter with an end that is not a date reads as one day', () {
    final outline = kitOutline(
      ChatOutline(chapters: [_chapter(endedAt: 'later')]),
    );
    final when = outline.chapters.single.when;
    expect(when, startsWith('Sep'));
    expect(when, isNot(contains('since')));
    expect(when, isNot(contains('–')));
  });

  test('message counts leave tool results out and follow the span', () {
    final messages = [
      _message('u1', 'user'),
      _message('a1', 'assistant'),
      _message('t1', 'tool'),
      _message('u2', 'user'),
      _message('a2', 'assistant'),
    ];
    final outline = kitOutline(
      ChatOutline(
        chapters: [
          _chapter(first: 'u1', last: 't1'),
          _chapter(id: 'B', title: 'B', first: 'u2', last: 'a2'),
          _chapter(id: 'C', title: 'C', first: 'gone', last: 'a2'),
          _chapter(id: 'D', title: 'D', first: 'x', last: 'y'),
        ],
      ),
      messages: messages,
    );
    expect([for (final c in outline.chapters) c.messageCount], [2, 2, 1, 0]);
  });
}
